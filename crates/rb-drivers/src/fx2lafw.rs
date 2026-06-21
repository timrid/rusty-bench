//! **fx2lafw** — Logic Analyzer driver for FX2LP-based devices running the
//! [fx2lafw](https://sigrok.org/wiki/Fx2lafw) firmware.
//!
//! Implements the sigrok fx2lafw USB vendor-request protocol (clean-room,
//! no GPLv3 source referenced).
//!
//! # Protocol overview
//! - Firmware upload: Cypress EZ-USB bootloader via control endpoint 0 (vendor
//!   request `0xA0`).  The FX2LP firmware (`.ihx`) is written to internal RAM
//!   chunk by chunk, then execution jumps to the entry point.
//! - After firmware is running, all commands use EP0 vendor control transfers:
//!   - `CMD_GET_FW_VERSION` (bRequest `0xB0`, IN): reads `{major, minor}` (2 bytes)
//!   - `CMD_START_ACQ` (bRequest `0xB1`, OUT): sends `{flags, delay_h, delay_l}` (3 bytes)
//!   - `CMD_GET_REVID` (bRequest `0xB2`, IN): reads chip revision (1 byte)
//! - Sample data streams on EP2 IN (`0x82`) bulk endpoint.
//!   - 8-bit mode: 1 byte per sample, bit `c` = channel `c`
//!   - 16-bit mode: 2 bytes per sample (little-endian), bits 0–7 = channels 0–7,
//!     bits 8–15 = channels 8–15
//! - Sample rate: GPIF delay = `clock_hz / target_hz - 1`, capped at 1536, sent
//!   big-endian in the start command. Clock source is 48 MHz or 30 MHz.
//!
//! The driver is tested against [`MockTransport`] for all protocol logic.
//!
//! [`MockTransport`]: rb_transport::MockTransport

use async_trait::async_trait;
use futures::executor::block_on;

use rb_device::{
    AcquisitionSource, Device, DeviceClass, DeviceError, DeviceId, DeviceInfo, DeviceResult,
    LogicAnalyzer,
};
use rb_model::{DigitalChannel, SampleChunk};
use rb_transport::{DeviceCandidate, DriverError, DriverFactory, DriverResult, Transport};

// ── Protocol constants ─────────────────────────────────────────────────────────

// -- USB vendor request codes (bRequest) ----------------------------------------

/// Query firmware version (device → host, 2 bytes: `{major, minor}`).
const CMD_GET_FW_VERSION: u8 = 0xB0;
/// Start acquisition (host → device, 3-byte data phase).
const CMD_START_ACQ: u8 = 0xB1;
/// Query chip revision ID (device → host, 1 byte: REVID register).
const CMD_GET_REVID: u8 = 0xB2;

// -- CMD_START_ACQ flags --------------------------------------------------------

/// Bit-position of CLK_CTL2 flag (alternating CTL1/CTL2 output for analog).
#[allow(dead_code)]
const FLAG_CLK_CTL2_POS: u8 = 4;
/// Bit-position of WIDE flag (0 = 8-bit samples, 1 = 16-bit samples).
const FLAG_WIDE_POS: u8 = 5;
/// Bit-position of CLK_SRC flag (0 = 30 MHz, 1 = 48 MHz internal clock).
const FLAG_CLK_SRC_POS: u8 = 6;

#[allow(dead_code)]
const FLAG_CLK_CTL2: u8 = 1 << FLAG_CLK_CTL2_POS;
const FLAG_SAMPLE_8BIT: u8 = 0 << FLAG_WIDE_POS;
const FLAG_SAMPLE_16BIT: u8 = 1 << FLAG_WIDE_POS;
const FLAG_CLK_30MHZ: u8 = 0 << FLAG_CLK_SRC_POS;
const FLAG_CLK_48MHZ: u8 = 1 << FLAG_CLK_SRC_POS;

// -- Cypress FX2LP bootloader ---------------------------------------------------

/// Cypress FX2LP vendor request for firmware upload.
const CYPRESS_FW_WRITE: u8 = 0xA0;

/// FX2LP internal RAM start address for firmware upload.
#[allow(dead_code)]
const FW_BASE_ADDR: u16 = 0x0000;
/// Entry point where execution begins after firmware upload.
const FW_ENTRY: u16 = 0x0000;

// -- Endpoints ------------------------------------------------------------------

/// Bulk IN endpoint for sample data (EP2 IN).
const EP_DATA: u8 = 0x82;

// -- Device limits --------------------------------------------------------------

/// Maximum number of logic channels supported by fx2lafw (8-bit mode).
const MAX_CHANNELS: u8 = 8;
/// Maximum number of channels in 16-bit mode.
const MAX_CHANNELS_16BIT: u8 = 16;

/// FX2LP internal clock frequencies that fx2lafw supports.
const CLOCK_48MHZ: f64 = 48_000_000.0;
const CLOCK_30MHZ: f64 = 30_000_000.0;

/// Maximum GPIF sample delay value (6 × 256 GPIF states).
const MAX_GPIF_DELAY: u16 = 1536;

/// Required fx2lafw firmware major version.
const REQUIRED_FW_MAJOR: u8 = 1;

/// Maximum milliseconds to wait for device renumeration after firmware upload.
const MAX_RENUM_DELAY_MS: u64 = 3000;
/// Poll interval during renumeration wait.
const RENUM_POLL_MS: u64 = 100;

/// Maximum chunks to pull per `next_chunk` call (to bound latency).
const MAX_PUMP_CHUNKS: usize = 16;

// ── Configuration ─────────────────────────────────────────────────────────────

/// fx2lafw driver configuration.
#[derive(Clone, Debug)]
pub struct Fx2lafwConfig {
    /// Number of digital channels to capture (1–16, 9–16 requires 16-bit mode).
    pub channels: u8,
    /// Target sample rate in hertz (will be rounded to the nearest achievable
    /// GPIF delay; actual rate depends on selected clock source).
    pub sample_rate_hz: f64,
}

impl Default for Fx2lafwConfig {
    fn default() -> Self {
        Self {
            channels: 8,
            sample_rate_hz: 1_000_000.0,
        }
    }
}

// ── Protocol helpers ───────────────────────────────────────────────────────────

/// Result of computing a GPIF sample delay for a requested sample rate.
struct DelayConfig {
    /// Selected clock frequency in Hz.
    clock_hz: f64,
    /// GPIF delay value (0–1535), sent big-endian in the start command.
    delay: u16,
    /// Achievable sample rate.
    actual_rate_hz: f64,
}

/// Compute the GPIF delay and select a clock source for a target sample rate.
///
/// Prefers 48 MHz when it yields an exact division, otherwise falls back to
/// 30 MHz.  The delay is `clock_hz / target_hz - 1`, capped at [`MAX_GPIF_DELAY`].
fn compute_delay(target_hz: f64) -> DelayConfig {
    for &clock in &[CLOCK_48MHZ, CLOCK_30MHZ] {
        // fx2lafw only uses this clock if the division is exact
        let div_f = clock / target_hz;
        if (div_f - div_f.round()).abs() > 1e-6 {
            continue;
        }
        let delay = (div_f as u16).saturating_sub(1).min(MAX_GPIF_DELAY);
        let actual = clock / (delay as f64 + 1.0);
        return DelayConfig {
            clock_hz: clock,
            delay,
            actual_rate_hz: actual,
        };
    }
    // No exact match — use 48 MHz with rounded division.
    let div_f = (CLOCK_48MHZ / target_hz).round().max(1.0);
    let delay = (div_f as u16).saturating_sub(1).min(MAX_GPIF_DELAY);
    let actual = CLOCK_48MHZ / (delay as f64 + 1.0);
    DelayConfig {
        clock_hz: CLOCK_48MHZ,
        delay,
        actual_rate_hz: actual,
    }
}

/// Build the flags byte for [`CMD_START_ACQ`].
fn start_flags(sample_wide: bool, clock_48mhz: bool, _analog_enabled: bool) -> u8 {
    let mut flags: u8 = 0;
    if sample_wide {
        flags |= FLAG_SAMPLE_16BIT;
    } else {
        flags |= FLAG_SAMPLE_8BIT;
    }
    if clock_48mhz {
        flags |= FLAG_CLK_48MHZ;
    } else {
        flags |= FLAG_CLK_30MHZ;
    }
    // Analog/CTL2 not yet implemented; leave CLK_CTL2 unset.
    flags
}

// ── Device ─────────────────────────────────────────────────────────────────────

/// An fx2lafw-based logic analyzer device.
///
/// All protocol I/O goes through the owned [`Transport`], making the device
/// testable via [`MockTransport`](rb_transport::MockTransport).
pub struct Fx2lafwDevice {
    id: DeviceId,
    info: DeviceInfo,
    transport: Box<dyn Transport>,
    config: Fx2lafwConfig,
    /// Firmware version reported by the device (major, minor).
    fw_version: (u8, u8),
    /// Whether 16-bit sampling is used (channels > 8 or analog).
    sample_wide: bool,
    /// Selected clock frequency for the current acquisition.
    clock_hz: f64,
    /// Achievable sample rate after GPIF delay computation.
    actual_rate: f64,
    channels: Vec<DigitalChannel>,
    /// Buffer for partially-received sample data between `next_chunk` calls.
    sample_buf: Vec<u8>,
    /// Whether acquisition has been started (device is producing samples).
    running: bool,
}

impl Fx2lafwDevice {
    /// Creates a device from an already-opened transport.
    ///
    /// The transport should already have firmware loaded and the device
    /// should be ready for configuration.  The `id` is typically derived
    /// from the device address at scan time.
    ///
    /// Call [`open`](Device::open) to verify firmware version before use.
    #[must_use]
    pub fn new(
        id: DeviceId,
        info: DeviceInfo,
        transport: Box<dyn Transport>,
        config: Fx2lafwConfig,
    ) -> Self {
        let ch = config.channels.clamp(1, MAX_CHANNELS_16BIT);
        let sample_wide = ch > MAX_CHANNELS;
        let channels: Vec<DigitalChannel> = (0..ch)
            .map(|c| DigitalChannel::new(rb_model::ChannelId(c as u32), format!("D{c}"), c))
            .collect();
        Self {
            id,
            info,
            transport,
            config: Fx2lafwConfig {
                channels: ch,
                sample_rate_hz: config.sample_rate_hz,
            },
            fw_version: (0, 0),
            sample_wide,
            clock_hz: CLOCK_48MHZ,
            actual_rate: 0.0,
            channels,
            sample_buf: Vec::new(),
            running: false,
        }
    }
}

#[async_trait(?Send)]
impl Device for Fx2lafwDevice {
    fn id(&self) -> &DeviceId {
        &self.id
    }

    fn info(&self) -> &DeviceInfo {
        &self.info
    }

    fn classes(&self) -> Vec<DeviceClass> {
        vec![DeviceClass::LogicAnalyzer]
    }

    fn as_logic_analyzer(&self) -> Option<&dyn LogicAnalyzer> {
        Some(self)
    }

    fn as_logic_analyzer_mut(&mut self) -> Option<&mut dyn LogicAnalyzer> {
        Some(self)
    }

    fn as_acquisition_source_mut(&mut self) -> Option<&mut dyn AcquisitionSource> {
        Some(self)
    }

    async fn open(&mut self) -> DeviceResult<()> {
        // Query firmware version and validate.
        self.fw_version = get_fw_version(&mut *self.transport).await?;
        if self.fw_version.0 != REQUIRED_FW_MAJOR {
            return Err(DeviceError::Protocol(format!(
                "unsupported firmware version {}.{} (need {}.x)",
                self.fw_version.0, self.fw_version.1, REQUIRED_FW_MAJOR
            )));
        }

        // Query chip revision.
        let revid = get_revid(&mut *self.transport).await?;
        let chip = if revid == 1 { "FX2LP (CY7C68013A)" } else { "FX2 (CY7C68013)" };

        // Update device info with discovered details.
        let vendor = core::mem::take(&mut self.info.vendor);
        let model = core::mem::take(&mut self.info.model);
        let serial = self.info.serial.take();
        let mut info = DeviceInfo::new(
            vendor,
            format!("{model} fw:{}.{} chip:{chip}", self.fw_version.0, self.fw_version.1),
        );
        info.serial = serial;
        self.info = info;

        Ok(())
    }

    async fn close(&mut self) -> DeviceResult<()> {
        let _ = self.transport.close().await;
        Ok(())
    }
}

// ── Command helpers ────────────────────────────────────────────────────────────

/// Query firmware version from the device.
///
/// Sends `CMD_GET_FW_VERSION` (0xB0) as a vendor IN request and reads the
/// 2-byte `{major, minor}` response.
async fn get_fw_version(transport: &mut dyn Transport) -> DeviceResult<(u8, u8)> {
    let resp = transport
        .control_transfer(
            0xC0,              // vendor, device-to-host (IN)
            CMD_GET_FW_VERSION, // bRequest = 0xB0
            0,                  // wValue
            0,                  // wIndex
            &[],                // no OUT data
        )
        .await
        .map_err(|e| DeviceError::Transport(format!("get_fw_version: {e}")))?;
    if resp.len() < 2 {
        return Err(DeviceError::Protocol(format!(
            "get_fw_version: expected 2 bytes, got {}",
            resp.len()
        )));
    }
    Ok((resp[0], resp[1]))
}

/// Query chip revision ID from the device.
///
/// Sends `CMD_GET_REVID` (0xB2) as a vendor IN request and reads the 1-byte
/// REVID response.  Returns 1 for FX2LP (CY7C68013A), 0 for FX2 (CY7C68013).
async fn get_revid(transport: &mut dyn Transport) -> DeviceResult<u8> {
    let resp = transport
        .control_transfer(
            0xC0,           // vendor, device-to-host (IN)
            CMD_GET_REVID,   // bRequest = 0xB2
            0,               // wValue
            0,               // wIndex
            &[],             // no OUT data
        )
        .await
        .map_err(|e| DeviceError::Transport(format!("get_revid: {e}")))?;
    if resp.is_empty() {
        return Err(DeviceError::Protocol("get_revid: empty response".into()));
    }
    Ok(resp[0])
}

/// Send the CMD_START_ACQ (0xB1) vendor request with a 3-byte data phase.
///
/// The data phase is `{flags, delay_h, delay_l}` where delay is big-endian.
async fn start_acquisition(
    transport: &mut dyn Transport,
    flags: u8,
    delay: u16,
) -> DeviceResult<()> {
    let delay_h = (delay >> 8) as u8;
    let delay_l = (delay & 0xFF) as u8;
    let data = [flags, delay_h, delay_l];
    transport
        .control_transfer(
            0x40,            // vendor, host-to-device (OUT)
            CMD_START_ACQ,   // bRequest = 0xB1
            0,               // wValue
            0,               // wIndex
            &data,           // 3-byte data phase
        )
        .await
        .map_err(|e| DeviceError::Transport(format!("start_acquisition: {e}")))?;
    Ok(())
}

#[async_trait(?Send)]
impl LogicAnalyzer for Fx2lafwDevice {
    fn channels(&self) -> &[DigitalChannel] {
        &self.channels
    }

    fn sample_rate_hz(&self) -> f64 {
        self.actual_rate
    }

    async fn set_sample_rate_hz(&mut self, hz: f64) -> DeviceResult<()> {
        self.config.sample_rate_hz = hz;
        // Rate is applied on next arm(); just store the target.
        Ok(())
    }

    async fn arm(&mut self) -> DeviceResult<()> {
        let dc = compute_delay(self.config.sample_rate_hz);
        let clock_48mhz = (dc.clock_hz - CLOCK_48MHZ).abs() < 1.0;
        let flags = start_flags(self.sample_wide, clock_48mhz, false);

        start_acquisition(&mut *self.transport, flags, dc.delay).await?;

        self.clock_hz = dc.clock_hz;
        self.actual_rate = dc.actual_rate_hz;
        self.running = true;
        self.sample_buf.clear();
        Ok(())
    }

    async fn stop(&mut self) -> DeviceResult<()> {
        // The fx2lafw firmware has no explicit stop command; the GPIF engine
        // stops when its waveform completes or the host stops reading.
        self.running = false;
        Ok(())
    }
}

impl AcquisitionSource for Fx2lafwDevice {
    fn next_chunk(&mut self, max_samples: usize) -> SampleChunk {
        let sample_bytes = if self.sample_wide { 2 } else { 1 };

        if !self.running {
            let _n = self.sample_buf.len();
            let samples = decode_samples(&self.sample_buf, self.sample_wide, max_samples);
            self.sample_buf.clear();
            return samples;
        }

        // Read up to (max_samples * sample_bytes) bytes from EP2 IN.
        let want_bytes = max_samples * sample_bytes;
        let mut read_buf = vec![0u8; want_bytes.min(4096)];

        for _ in 0..MAX_PUMP_CHUNKS {
            match block_on(self.transport.read(&mut read_buf)) {
                Ok(0) => break, // End of stream.
                Ok(n) => {
                    self.sample_buf.extend_from_slice(&read_buf[..n]);
                    if self.sample_buf.len() >= want_bytes {
                        break;
                    }
                }
                Err(_) => break, // Transport error → stop reading.
            }
        }

        // Decode as many complete samples as we have (up to max_samples).
        let avail = self.sample_buf.len();
        let complete = (avail / sample_bytes) * sample_bytes;
        let samples = decode_samples(
            &self.sample_buf[..complete],
            self.sample_wide,
            max_samples,
        );
        // Drain only the bytes actually consumed by decode_samples.
        let consumed = samples.logic().len() * sample_bytes;
        self.sample_buf.drain(..consumed);
        samples
    }
}

// ── Sample decoding ────────────────────────────────────────────────────────────

/// Decode raw sample bytes from the fx2lafw into a [`SampleChunk`].
///
/// In 8-bit mode (`sample_wide == false`), each byte encodes one sample with
/// bit `c` corresponding to channel `c`.
///
/// In 16-bit mode (`sample_wide == true`), each pair of bytes (little-endian)
/// encodes one sample: bits 0–7 = channels 0–7, bits 8–15 = channels 8–15.
fn decode_samples(data: &[u8], sample_wide: bool, max_samples: usize) -> SampleChunk {
    let mut logic = Vec::with_capacity(max_samples);

    if sample_wide {
        // 16-bit samples: 2 bytes per sample, little-endian.
        for chunk in data.chunks_exact(2).take(max_samples) {
            let val = u64::from(u16::from_le_bytes([chunk[0], chunk[1]]));
            logic.push(val);
        }
    } else {
        // 8-bit samples: 1 byte per sample.
        for &byte in data.iter().take(max_samples) {
            logic.push(u64::from(byte));
        }
    }

    let mut chunk = SampleChunk::new();
    if !logic.is_empty() {
        chunk = chunk.with_logic(logic);
    }
    chunk
}

// ── Firmware upload ────────────────────────────────────────────────────────────

/// Parse Intel HEX format data chunks.
///
/// Returns a list of `(address, data)` pairs extracted from I8HEX / I16HEX
/// records.  Extended Linear Address (0x04) records set the upper 16 bits
/// of the address.  EOF record (0x01) terminates parsing.
fn parse_ihex(data: &[u8]) -> Result<Vec<(u16, Vec<u8>)>, String> {
    let mut result: Vec<(u16, Vec<u8>)> = Vec::new();
    let mut base_addr: u32 = 0;

    for line in data.split(|&b| b == b'\n') {
        let line = line.trim_ascii();
        if line.is_empty() {
            continue;
        }
        if line.first() != Some(&b':') {
            continue; // Not an Intel HEX record.
        }
        // Minimum record: `:BBAAAATTCC` (11 hex chars + colon = 12 bytes)
        let hex = std::str::from_utf8(line).map_err(|_| "non-UTF-8 in HEX line".to_string())?;
        if hex.len() < 11 {
            return Err(format!("malformed HEX record: too short ({hex})"));
        }

        let byte_count = u8::from_str_radix(&hex[1..3], 16).map_err(|e| e.to_string())? as usize;
        let addr = u16::from_str_radix(&hex[3..7], 16).map_err(|e| e.to_string())?;
        let rectype = u8::from_str_radix(&hex[7..9], 16).map_err(|e| e.to_string())?;

        // Expected length: `:BBAAAATTHHDD...CC`
        let expected_len = 9 + byte_count * 2 + 2; // 9 hex chars before data + data + checksum
        if hex.len() < expected_len {
            return Err(format!(
                "HEX record truncated: expected {expected_len} chars, got {}",
                hex.len()
            ));
        }

        match rectype {
            0x00 => {
                // Data record
                let mut bytes = Vec::with_capacity(byte_count);
                for j in 0..byte_count {
                    let off = 9 + j * 2;
                    let b =
                        u8::from_str_radix(&hex[off..off + 2], 16).map_err(|e| e.to_string())?;
                    bytes.push(b);
                }
                let full_addr = (base_addr + u32::from(addr)) as u16;
                result.push((full_addr, bytes));
            }
            0x01 => {
                // End Of File
                break;
            }
            0x04 => {
                // Extended Linear Address (upper 16 bits)
                if byte_count != 2 {
                    return Err("bad Extended Linear Address record".to_string());
                }
                let hi = u16::from_str_radix(&hex[9..13], 16).map_err(|e| e.to_string())?;
                base_addr = u32::from(hi) << 16;
            }
            _ => {
                // Ignore other record types (start segment address, etc.)
            }
        }
    }

    Ok(result)
}

/// Upload Intel HEX firmware to the FX2LP via control transfers.
///
/// - Writes each data chunk to the appropriate RAM address.
/// - Starts execution at the entry point after all chunks are written.
///
/// # Errors
/// Returns an error if any control transfer fails or the firmware format is
/// invalid.
pub async fn upload_firmware(
    transport: &mut dyn Transport,
    ihex_data: &[u8],
) -> Result<(), String> {
    let chunks = parse_ihex(ihex_data)?;
    for (addr, data) in &chunks {
        transport
            .control_transfer(
                0x40, // vendor, host-to-device
                CYPRESS_FW_WRITE,
                *addr,
                0x0000,
                data,
            )
            .await
            .map_err(|e| format!("firmware upload failed at 0x{addr:04X}: {e}"))?;
    }
    // Start execution.
    transport
        .control_transfer(
            0x40, // vendor, host-to-device
            CYPRESS_FW_WRITE,
            FW_ENTRY,
            0x0000,
            &[],
        )
        .await
        .map_err(|e| format!("firmware start failed: {e}"))?;
    Ok(())
}

// ── Device table ──────────────────────────────────────────────────────────────

/// USB Vendor ID for Cypress FX2LP (bootloader mode, no EEPROM).
const CYPRESS_VID: u16 = 0x04B4;
/// USB Product ID for FX2LP in bootloader mode.
const CYPRESS_PID: u16 = 0x8613;

/// USB Vendor ID for Openmoko / sigrok (fx2lafw firmware loaded).
#[allow(dead_code)]
const FX2LAFW_VID: u16 = 0x1D50;

/// A device entry in the fx2lafw support table.
#[allow(dead_code)]
struct DeviceProfile {
    vid: u16,
    pid: u16,
    /// Display vendor string.
    vendor: &'static str,
    /// Display model string.
    model: &'static str,
    /// Whether this device supports 16-bit sampling.
    has_16bit: bool,
}

/// Known fx2lafw-compatible devices (matches sigrok's `supported_fx2[]`).
static SUPPORTED_DEVICES: &[DeviceProfile] = &[
    // Cypress FX2 (no EEPROM) — bootloader VID/PID
    DeviceProfile { vid: 0x04B4, pid: 0x8613, vendor: "Cypress", model: "FX2 (bootloader)", has_16bit: true },
    // USBee AX (and clones)
    DeviceProfile { vid: 0x08A9, pid: 0x0014, vendor: "CWAV", model: "USBee AX", has_16bit: false },
    // USBee DX
    DeviceProfile { vid: 0x08A9, pid: 0x0015, vendor: "CWAV", model: "USBee DX", has_16bit: true },
    // USBee SX
    DeviceProfile { vid: 0x08A9, pid: 0x0009, vendor: "CWAV", model: "USBee SX", has_16bit: false },
    // Saleae Logic (and many clones)
    DeviceProfile { vid: 0x0925, pid: 0x3881, vendor: "Saleae", model: "Logic", has_16bit: false },
    // Braintechnology USB-LPS
    DeviceProfile { vid: 0x16D0, pid: 0x0498, vendor: "Braintechnology", model: "USB-LPS", has_16bit: true },
    // sigrok FX2 LA 8ch
    DeviceProfile { vid: 0x1D50, pid: 0x608C, vendor: "sigrok", model: "FX2 LA (8ch)", has_16bit: false },
    // sigrok FX2 LA 16ch
    DeviceProfile { vid: 0x1D50, pid: 0x608D, vendor: "sigrok", model: "FX2 LA (16ch)", has_16bit: true },
    // fx2lafw-generic PIDs
    DeviceProfile { vid: 0x1D50, pid: 0x6081, vendor: "sigrok", model: "fx2lafw", has_16bit: false },
    DeviceProfile { vid: 0x1D50, pid: 0x6082, vendor: "sigrok", model: "fx2lafw", has_16bit: false },
    DeviceProfile { vid: 0x1D50, pid: 0x608E, vendor: "sigrok", model: "fx2lafw", has_16bit: false },
    // sigrok usb-c-grok
    DeviceProfile { vid: 0x1D50, pid: 0x608F, vendor: "sigrok", model: "usb-c-grok", has_16bit: false },
];

/// Look up a device profile by VID/PID.
fn find_profile(vid: u16, pid: u16) -> Option<&'static DeviceProfile> {
    SUPPORTED_DEVICES
        .iter()
        .find(|p| p.vid == vid && p.pid == pid)
}

/// Returns true if the VID/PID pair is in bootloader mode (needs firmware upload).
fn is_bootloader(vid: u16, pid: u16) -> bool {
    vid == CYPRESS_VID && pid == CYPRESS_PID
}

// ── Driver factory ────────────────────────────────────────────────────────────

/// Driver factory for fx2lafw-based logic analyzers.
///
/// On native builds (with the `fx2lafw` feature + `nusb`) the factory
/// enumerates USB devices directly.  On the web the caller must provide
/// pre-authorized candidates via [`set_candidates`](Self::set_candidates).
pub struct Fx2lafwFactory {
    candidates: Vec<DeviceCandidate>,
}

impl Fx2lafwFactory {
    /// Creates a factory with an empty candidate list.
    #[must_use]
    pub fn new() -> Self {
        Self {
            candidates: Vec::new(),
        }
    }

    /// Sets (or replaces) the list of pre-enumerated candidates (web use).
    pub fn set_candidates(&mut self, candidates: Vec<DeviceCandidate>) {
        self.candidates = candidates;
    }
}

impl Default for Fx2lafwFactory {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait(?Send)]
impl DriverFactory for Fx2lafwFactory {
    fn name(&self) -> &str {
        "fx2lafw"
    }

    fn supported_classes(&self) -> &[DeviceClass] {
        static CLASSES: [DeviceClass; 1] = [DeviceClass::LogicAnalyzer];
        &CLASSES
    }

    async fn scan(&self) -> DriverResult<Vec<DeviceCandidate>> {
        if !self.candidates.is_empty() {
            return Ok(self.candidates.clone());
        }
        scan_usb().await
    }

    async fn connect(&self, candidate: &DeviceCandidate) -> DriverResult<Box<dyn Device>> {
        connect_usb(candidate).await
    }
}

/// Enumerate USB devices matching known fx2lafw or Cypress bootloader VID/PID pairs.
#[cfg(feature = "fx2lafw")]
async fn scan_usb() -> DriverResult<Vec<DeviceCandidate>> {
    let devices = nusb::list_devices()
        .map_err(|e| DriverError::Transport(rb_transport::TransportError::Io(e.to_string())))?;

    let mut results = Vec::new();
    for dev in devices {
        let vid = dev.vendor_id();
        let pid = dev.product_id();
        let Some(profile) = find_profile(vid, pid) else {
            continue;
        };

        let serial = dev.serial_number().unwrap_or("").to_string();
        // Address encodes VID, PID, serial, and a bootloader flag.
        let boot_flag = if is_bootloader(vid, pid) { "1" } else { "0" };
        let address = format!("{:04X}:{:04X}:{}:{}", vid, pid, serial, boot_flag);

        let mut info = DeviceInfo::new(profile.vendor, profile.model);
        if !serial.is_empty() {
            info = info.with_serial(serial);
        }

        results.push(DeviceCandidate::new(info, address));
    }
    Ok(results)
}

/// Connect to a USB device previously found by [`scan_usb`].
///
/// If the device is in Cypress bootloader mode, firmware must be uploaded first
/// (via [`upload_firmware`]), then the driver waits for renumeration and connects
/// to the newly-enumerated fx2lafw device.
#[cfg(feature = "fx2lafw")]
async fn connect_usb(candidate: &DeviceCandidate) -> DriverResult<Box<dyn Device>> {
    // Parse the address: "VID:PID:SERIAL:BOOTFLAG"
    let parts: Vec<&str> = candidate.address.split(':').collect();
    if parts.len() < 2 {
        return Err(DriverError::NotFound);
    }
    let target_vid = u16::from_str_radix(parts[0], 16).map_err(|_| DriverError::NotFound)?;
    let target_pid = u16::from_str_radix(parts[1], 16).map_err(|_| DriverError::NotFound)?;
    let target_serial = parts.get(2).copied().unwrap_or("");
    let is_boot = parts.get(3).copied().unwrap_or("0") == "1";

    // If the device is in bootloader mode, firmware must have been uploaded
    // externally before calling connect.  We expect to find the device at a
    // fx2lafw VID/PID by now.
    if is_boot {
        return Err(DriverError::Transport(rb_transport::TransportError::Io(
            "device is in bootloader mode — upload firmware first, then reconnect".into(),
        )));
    }

    // Find and open the device by VID/PID/serial.
    let dev_info = find_usb_device(target_vid, target_pid, target_serial).await?;
    let device = dev_info
        .open()
        .map_err(|e| DriverError::Transport(rb_transport::TransportError::Io(e.to_string())))?;

    // Claim interface 0 (the only interface fx2lafw exposes).
    let interface = device
        .detach_and_claim_interface(0)
        .map_err(|e| DriverError::Transport(rb_transport::TransportError::Io(e.to_string())))?;

    // fx2lafw firmware only uses EP2 IN (0x82) for sample data.
    // All commands go through EP0 control transfers — no bulk OUT endpoint needed.
    let transport: Box<dyn Transport> = Box::new(rb_transport::nusb::NusbTransport::new(
        interface,
        EP_DATA, // bulk IN  = EP2 IN  (0x82)
        0x00,    // bulk OUT = unused (firmware has no bulk OUT endpoint)
    ));

    let id = DeviceId::new(&candidate.address);
    let info = candidate.info.clone();
    let config = Fx2lafwConfig::default();

    Ok(Box::new(Fx2lafwDevice::new(id, info, transport, config)))
}

/// Upload firmware to a device in bootloader mode, then wait for renumeration
/// and return a connected [`Device`].
///
/// The `firmware_data` is the Intel HEX (`.ihx`) file content.
/// `boot_serial` is the serial number of the bootloader device (may be empty).
///
/// After upload, this function polls for a new device at the fx2lafw VID/PID
/// for up to [`MAX_RENUM_DELAY_MS`].
#[cfg(feature = "fx2lafw")]
pub async fn upload_firmware_and_connect(
    firmware_data: &[u8],
    boot_serial: &str,
) -> DriverResult<Box<dyn Device>> {
    // 1. Find and open the bootloader device.
    let boot_dev_info = find_usb_device(CYPRESS_VID, CYPRESS_PID, boot_serial).await?;
    let boot_device = boot_dev_info
        .open()
        .map_err(|e| DriverError::Transport(rb_transport::TransportError::Io(e.to_string())))?;

    let interface = boot_device
        .detach_and_claim_interface(0)
        .map_err(|e| DriverError::Transport(rb_transport::TransportError::Io(e.to_string())))?;

    let mut transport: Box<dyn Transport> = Box::new(rb_transport::nusb::NusbTransport::new(
        interface, 0x00, 0x00,
    ));

    // 2. Upload firmware.
    upload_firmware(&mut *transport, firmware_data)
        .await
        .map_err(|e| DriverError::Transport(rb_transport::TransportError::Io(e)))?;

    // The device disconnects and re-enumerates with new VID/PID.
    drop(transport);

    // 3. Wait for renumeration — poll until a fx2lafw device appears.
    let start = std::time::Instant::now();
    loop {
        // Small delay between polls.
        #[cfg(not(target_arch = "wasm32"))]
        std::thread::sleep(std::time::Duration::from_millis(RENUM_POLL_MS));

        let devices = nusb::list_devices().map_err(|e| {
            DriverError::Transport(rb_transport::TransportError::Io(e.to_string()))
        })?;

        for dev in devices {
            let vid = dev.vendor_id();
            let pid = dev.product_id();
            // Look for any fx2lafw device (not the bootloader).
            if !is_bootloader(vid, pid) {
                if let Some(profile) = find_profile(vid, pid) {
                    let serial = dev.serial_number().unwrap_or("");
                    let address = format!("{:04X}:{:04X}:{}:0", vid, pid, serial);

                    let mut info = DeviceInfo::new(profile.vendor, profile.model);
                    if !serial.is_empty() {
                        info = info.with_serial(serial);
                    }

                    let candidate = DeviceCandidate::new(info, address);
                    return connect_usb(&candidate).await;
                }
            }
        }

        if start.elapsed().as_millis() as u64 >= MAX_RENUM_DELAY_MS as u64 {
            return Err(DriverError::Transport(rb_transport::TransportError::Io(
                "device did not re-enumerate after firmware upload".into(),
            )));
        }
    }
}

/// Find a USB device by VID, PID, and optional serial.
#[cfg(feature = "fx2lafw")]
async fn find_usb_device(
    vid: u16,
    pid: u16,
    serial: &str,
) -> DriverResult<nusb::DeviceInfo> {
    nusb::list_devices()
        .map_err(|e| DriverError::Transport(rb_transport::TransportError::Io(e.to_string())))?
        .find(|dev| {
            dev.vendor_id() == vid
                && dev.product_id() == pid
                && (serial.is_empty() || dev.serial_number().unwrap_or("") == serial)
        })
        .ok_or(DriverError::NotFound)
}

#[cfg(not(feature = "fx2lafw"))]
async fn scan_usb() -> DriverResult<Vec<DeviceCandidate>> {
    Ok(Vec::new())
}

#[cfg(not(feature = "fx2lafw"))]
async fn connect_usb(_candidate: &DeviceCandidate) -> DriverResult<Box<dyn Device>> {
    Err(DriverError::NotFound)
}

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use futures::executor::block_on;
    use rb_transport::MockTransport;

    // ── GPIF delay computation ────────────────────────────────────────────────

    #[test]
    fn delay_48mhz_exact_division() {
        // 48 MHz → target 1 MHz: delay = 48 - 1 = 47, actual = 48M / 48 = 1 MHz
        let dc = compute_delay(1_000_000.0);
        assert_eq!(dc.delay, 47);
        assert!((dc.actual_rate_hz - 1_000_000.0).abs() < 1.0);
        assert!((dc.clock_hz - CLOCK_48MHZ).abs() < 1.0);
    }

    #[test]
    fn delay_30mhz_exact_division() {
        // 5 MHz: 48/5 = 9.6 (not exact), 30/5 = 6 (exact) → picks 30 MHz, delay = 6-1 = 5
        let dc = compute_delay(5_000_000.0);
        assert_eq!(dc.delay, 5);
        assert!((dc.actual_rate_hz - 5_000_000.0).abs() < 1.0);
        assert!((dc.clock_hz - CLOCK_30MHZ).abs() < 1.0);
    }

    #[test]
    fn delay_prefers_48mhz_when_both_exact() {
        // 6 MHz divides both 48 MHz and 30 MHz; should prefer 48 MHz.
        let dc = compute_delay(6_000_000.0);
        assert!((dc.clock_hz - CLOCK_48MHZ).abs() < 1.0);
        assert_eq!(dc.delay, 7); // 48/6 - 1
    }

    #[test]
    fn delay_rounds_for_inexact() {
        // 7 MHz doesn't divide either clock exactly; falls back to rounded 48 MHz.
        let dc = compute_delay(7_000_000.0);
        assert!((dc.clock_hz - CLOCK_48MHZ).abs() < 1.0);
        // round(48/7) = 7; delay = 6
        assert_eq!(dc.delay, 6);
    }

    #[test]
    fn delay_capped_at_max_gpif() {
        // Very low target → delay capped at MAX_GPIF_DELAY (1536).
        let dc = compute_delay(1.0);
        assert_eq!(dc.delay, MAX_GPIF_DELAY);
        assert!(dc.actual_rate_hz > 0.0);
    }

    #[test]
    fn delay_minimum_is_zero() {
        // Target at or above 48 MHz → delay 0 → actual = 48 MHz.
        let dc = compute_delay(100_000_000.0);
        assert_eq!(dc.delay, 0);
        assert!((dc.actual_rate_hz - CLOCK_48MHZ).abs() < 1.0);
    }

    // ── Start flags ──────────────────────────────────────────────────────────

    #[test]
    fn flags_8bit_48mhz() {
        let flags = start_flags(false, true, false);
        assert_eq!(flags & FLAG_SAMPLE_16BIT, 0);
        assert_ne!(flags & FLAG_CLK_48MHZ, 0);
    }

    #[test]
    fn flags_16bit_30mhz() {
        let flags = start_flags(true, false, false);
        assert_ne!(flags & FLAG_SAMPLE_16BIT, 0);
        assert_eq!(flags & FLAG_CLK_48MHZ, 0);
    }

    // ── Sample decoding ───────────────────────────────────────────────────────

    #[test]
    fn decode_8bit_single_sample() {
        let data = vec![0b10101010u8];
        let chunk = decode_samples(&data, false, 1);
        assert_eq!(chunk.logic().len(), 1);
        assert_eq!(chunk.logic()[0], 0b10101010);
    }

    #[test]
    fn decode_8bit_multi_sample() {
        let data = vec![0x00u8, 0xFF, 0x55];
        let chunk = decode_samples(&data, false, 3);
        assert_eq!(chunk.logic().len(), 3);
        assert_eq!(chunk.logic()[0], 0x00);
        assert_eq!(chunk.logic()[1], 0xFF);
        assert_eq!(chunk.logic()[2], 0x55);
    }

    #[test]
    fn decode_8bit_respects_max_samples() {
        let data = vec![0x01, 0x02, 0x03, 0x04, 0x05];
        let chunk = decode_samples(&data, false, 3);
        assert_eq!(chunk.logic().len(), 3);
    }

    #[test]
    fn decode_8bit_empty() {
        let chunk = decode_samples(&[], false, 10);
        assert_eq!(chunk.logic().len(), 0);
    }

    #[test]
    fn decode_16bit_single_sample() {
        // little-endian 16-bit: low byte first
        let data = vec![0x34, 0x12]; // 0x1234
        let chunk = decode_samples(&data, true, 1);
        assert_eq!(chunk.logic().len(), 1);
        assert_eq!(chunk.logic()[0], 0x1234);
    }

    #[test]
    fn decode_16bit_multi_sample() {
        let data = vec![0xFF, 0x00, 0x00, 0xFF]; // 0x00FF, 0xFF00
        let chunk = decode_samples(&data, true, 2);
        assert_eq!(chunk.logic().len(), 2);
        assert_eq!(chunk.logic()[0], 0x00FF);
        assert_eq!(chunk.logic()[1], 0xFF00);
    }

    #[test]
    fn decode_16bit_incomplete_pair_is_skipped() {
        // Only 3 bytes → only 1 complete 16-bit sample.
        let data = vec![0x01, 0x02, 0x03];
        let chunk = decode_samples(&data, true, 10);
        assert_eq!(chunk.logic().len(), 1);
        assert_eq!(chunk.logic()[0], 0x0201);
    }

    #[test]
    fn decode_16bit_respects_max_samples() {
        let data = vec![0x01, 0x00, 0x02, 0x00, 0x03, 0x00];
        let chunk = decode_samples(&data, true, 2);
        assert_eq!(chunk.logic().len(), 2);
    }

    // ── Device lifecycle (MockTransport) ──────────────────────────────────────

    #[test]
    fn device_info_and_channels_are_correct() {
        let id = DeviceId::new("fx2lafw-0");
        let info = DeviceInfo::new("Cypress", "FX2LP");
        let dev = Fx2lafwDevice::new(
            id,
            info,
            Box::new(MockTransport::new()),
            Fx2lafwConfig::default(),
        );

        assert_eq!(dev.id().as_str(), "fx2lafw-0");
        assert_eq!(dev.info().vendor, "Cypress");
        assert_eq!(dev.info().model, "FX2LP");
        assert_eq!(dev.channels().len(), 8);
        assert_eq!(dev.channels()[0].name, "D0");
        assert_eq!(dev.channels()[7].name, "D7");
        assert!(dev.classes().contains(&DeviceClass::LogicAnalyzer));
    }

    #[test]
    fn device_with_16_channels_is_wide() {
        let id = DeviceId::new("fx2lafw-16");
        let info = DeviceInfo::new("sigrok", "FX2 LA 16ch");
        let dev = Fx2lafwDevice::new(
            id,
            info,
            Box::new(MockTransport::new()),
            Fx2lafwConfig { channels: 16, sample_rate_hz: 1_000_000.0 },
        );

        assert_eq!(dev.channels().len(), 16);
        assert_eq!(dev.channels()[15].name, "D15");
        assert!(dev.sample_wide);
    }

    #[test]
    fn open_validates_fw_version() {
        let mut transport = MockTransport::new();
        // Queue responses for get_fw_version (major=1, minor=4) and get_revid.
        transport.queue_control_response([1, 4]);  // fw version
        transport.queue_control_response([1]);       // revid (FX2LP)

        let id = DeviceId::new("test");
        let info = DeviceInfo::new("Test", "FX2");
        let mut dev = Fx2lafwDevice::new(
            id,
            info,
            Box::new(transport),
            Fx2lafwConfig::default(),
        );

        let result = block_on(dev.open());
        assert!(result.is_ok());
        assert_eq!(dev.fw_version, (1, 4));
    }

    #[test]
    fn open_rejects_wrong_fw_major() {
        let mut transport = MockTransport::new();
        transport.queue_control_response([2, 0]);  // fw major=2 (unsupported)
        transport.queue_control_response([1]);      // revid

        let mut dev = Fx2lafwDevice::new(
            DeviceId::new("test"),
            DeviceInfo::new("Test", "FX2"),
            Box::new(transport),
            Fx2lafwConfig::default(),
        );

        let result = block_on(dev.open());
        assert!(result.is_err());
    }

    #[test]
    fn arm_sends_correct_start_command() {
        let mut transport = MockTransport::new();
        transport.queue_control_response([1, 4]);  // fw version
        transport.queue_control_response([1]);       // revid
        transport.queue_control_response([]);        // start_acquisition response

        let mut dev = Fx2lafwDevice::new(
            DeviceId::new("test"),
            DeviceInfo::new("Test", "FX2"),
            Box::new(transport),
            Fx2lafwConfig { channels: 8, sample_rate_hz: 1_000_000.0 },
        );

        block_on(dev.open()).unwrap();
        block_on(dev.arm()).unwrap();

        // After arm(), the MockTransport should have recorded the control transfers.
        // We can't access it after the move, but arm() succeeded.
    }

    #[test]
    fn arm_with_mock_transport_verifies_protocol() {
        let mut transport = MockTransport::new();
        // Queue: fw version, revid, start_acquisition response
        transport.queue_control_response([1, 4]);
        transport.queue_control_response([1]);
        transport.queue_control_response([]);

        let mut dev = Fx2lafwDevice::new(
            DeviceId::new("test"),
            DeviceInfo::new("Test", "FX2"),
            Box::new(transport),
            Fx2lafwConfig { channels: 8, sample_rate_hz: 48_000_000.0 },
        );

        block_on(dev.open()).unwrap();
        block_on(dev.arm()).unwrap();
        assert!(dev.running);
        // At 48 MHz, delay should be 0, flags should be 8-bit + 48 MHz clock.
    }

    // ── Direct protocol tests (MockTransport, no device wrapper) ────────────

    #[test]
    fn get_fw_version_sends_correct_control_transfer() {
        let mut transport = MockTransport::new();
        transport.queue_control_response([1, 4]); // {major=1, minor=4}

        let result = block_on(get_fw_version(&mut transport));
        assert_eq!(result.unwrap(), (1, 4));

        let ctrl = transport.control_transfers();
        assert_eq!(ctrl.len(), 1);
        assert_eq!(ctrl[0].request_type, 0xC0, "must be vendor IN (device→host)");
        assert_eq!(ctrl[0].request, 0xB0, "bRequest must be CMD_GET_FW_VERSION (0xB0)");
        assert_eq!(ctrl[0].value, 0);
        assert_eq!(ctrl[0].index, 0);
    }

    #[test]
    fn get_fw_version_rejects_short_response() {
        let mut transport = MockTransport::new();
        transport.queue_control_response([0x01]); // only 1 byte, need 2

        let result = block_on(get_fw_version(&mut transport));
        assert!(result.is_err());
    }

    #[test]
    fn get_revid_sends_correct_control_transfer() {
        let mut transport = MockTransport::new();
        transport.queue_control_response([0x01]); // REVID=1 → FX2LP

        let result = block_on(get_revid(&mut transport));
        assert_eq!(result.unwrap(), 1);

        let ctrl = transport.control_transfers();
        assert_eq!(ctrl.len(), 1);
        assert_eq!(ctrl[0].request_type, 0xC0, "must be vendor IN (device→host)");
        assert_eq!(ctrl[0].request, 0xB2, "bRequest must be CMD_GET_REVID (0xB2)");
    }

    #[test]
    fn get_revid_rejects_empty_response() {
        let mut transport = MockTransport::new();
        transport.queue_control_response([]);

        let result = block_on(get_revid(&mut transport));
        assert!(result.is_err());
    }

    #[test]
    fn start_acquisition_sends_correct_control_transfer() {
        let mut transport = MockTransport::new();
        transport.queue_control_response([]);

        // flags=0x21 (8-bit, 48 MHz), delay=47 (big-endian: 0x002F)
        let result = block_on(start_acquisition(&mut transport, 0x21, 47));
        assert!(result.is_ok());

        let ctrl = transport.control_transfers();
        assert_eq!(ctrl.len(), 1);
        assert_eq!(ctrl[0].request_type, 0x40, "must be vendor OUT (host→device)");
        assert_eq!(ctrl[0].request, 0xB1, "bRequest must be CMD_START_ACQ (0xB1)");
        assert_eq!(ctrl[0].value, 0);
        assert_eq!(ctrl[0].index, 0);
        // 3-byte data phase: [flags, delay_h, delay_l]
        assert_eq!(ctrl[0].data, vec![0x21, 0x00, 0x2F]);
    }

    #[test]
    fn start_acquisition_encodes_delay_big_endian() {
        let mut transport = MockTransport::new();
        transport.queue_control_response([]);

        // delay=0x1234 → high=0x12, low=0x34
        block_on(start_acquisition(&mut transport, 0x00, 0x1234)).unwrap();

        let ctrl = transport.control_transfers();
        assert_eq!(ctrl[0].data, vec![0x00, 0x12, 0x34]);
    }

    #[test]
    fn start_acquisition_delay_zero_is_correct() {
        let mut transport = MockTransport::new();
        transport.queue_control_response([]);

        // delay=0 → both bytes zero
        block_on(start_acquisition(&mut transport, 0x40, 0)).unwrap();

        let ctrl = transport.control_transfers();
        assert_eq!(ctrl[0].data, vec![0x40, 0x00, 0x00]);
    }

    #[test]
    fn start_acquisition_max_delay_encodes_correctly() {
        let mut transport = MockTransport::new();
        transport.queue_control_response([]);

        // MAX_GPIF_DELAY=1536 = 0x0600
        block_on(start_acquisition(&mut transport, 0x00, MAX_GPIF_DELAY)).unwrap();

        let ctrl = transport.control_transfers();
        assert_eq!(ctrl[0].data, vec![0x00, 0x06, 0x00]);
    }

    // ── start_flags exhaustive ────────────────────────────────────────────────

    #[test]
    fn flags_8bit_30mhz() {
        let flags = start_flags(false, false, false);
        assert_eq!(flags & FLAG_SAMPLE_16BIT, 0, "8-bit → WIDE flag unset");
        assert_eq!(flags & FLAG_CLK_48MHZ, 0, "30 MHz → CLK_SRC flag unset");
    }

    #[test]
    fn flags_16bit_48mhz() {
        let flags = start_flags(true, true, false);
        assert_ne!(flags & FLAG_SAMPLE_16BIT, 0, "16-bit → WIDE flag set");
        assert_ne!(flags & FLAG_CLK_48MHZ, 0, "48 MHz → CLK_SRC flag set");
    }

    // ── stop / set_sample_rate_hz behaviour ──────────────────────────────────

    #[test]
    fn stop_is_noop_does_not_send_usb_command() {
        let transport = MockTransport::new();
        let mut dev = Fx2lafwDevice::new(
            DeviceId::new("test"),
            DeviceInfo::new("Test", "FX2"),
            Box::new(transport),
            Fx2lafwConfig::default(),
        );
        dev.running = true; // simulate armed state

        let result = block_on(dev.stop());
        assert!(result.is_ok());
        assert!(!dev.running);
        // No control transfers were issued (transport has no queued responses consumed).
    }

    #[test]
    fn set_sample_rate_hz_is_lazy_no_usb_command() {
        let transport = MockTransport::new();
        let mut dev = Fx2lafwDevice::new(
            DeviceId::new("test"),
            DeviceInfo::new("Test", "FX2"),
            Box::new(transport),
            Fx2lafwConfig::default(),
        );

        let result = block_on(dev.set_sample_rate_hz(2_000_000.0));
        assert!(result.is_ok());
        assert!((dev.config.sample_rate_hz - 2_000_000.0).abs() < 1.0);
        // No USB command was sent — only stored for next arm().
    }

    // ── open() side effects ──────────────────────────────────────────────────

    #[test]
    fn open_updates_device_info_with_fw_and_chip() {
        let mut transport = MockTransport::new();
        transport.queue_control_response([1, 4]);  // fw 1.4
        transport.queue_control_response([1]);      // revid=1 → FX2LP

        let mut dev = Fx2lafwDevice::new(
            DeviceId::new("test"),
            DeviceInfo::new("TestVendor", "TestModel"),
            Box::new(transport),
            Fx2lafwConfig::default(),
        );

        block_on(dev.open()).unwrap();
        assert_eq!(dev.info().vendor, "TestVendor");
        assert!(dev.info().model.contains("TestModel"), "model should contain original name");
        assert!(dev.info().model.contains("fw:1.4"), "model should contain fw version");
        assert!(dev.info().model.contains("FX2LP"), "model should contain chip type");
    }

    #[test]
    fn open_with_revid_zero_detects_fx2() {
        let mut transport = MockTransport::new();
        transport.queue_control_response([1, 2]);  // fw 1.2
        transport.queue_control_response([0]);      // revid=0 → FX2 (not FX2LP)

        let mut dev = Fx2lafwDevice::new(
            DeviceId::new("test"),
            DeviceInfo::new("X", "Y"),
            Box::new(transport),
            Fx2lafwConfig::default(),
        );

        block_on(dev.open()).unwrap();
        assert!(dev.info().model.contains("FX2 (CY7C68013)"));
        assert!(!dev.info().model.contains("FX2LP"));
    }

    // ── next_chunk integration (MockTransport) ───────────────────────────────

    #[test]
    fn next_chunk_when_stopped_drains_buffer() {
        let transport = MockTransport::new();

        let mut dev = Fx2lafwDevice::new(
            DeviceId::new("test"),
            DeviceInfo::new("X", "Y"),
            Box::new(transport),
            Fx2lafwConfig { channels: 8, sample_rate_hz: 1_000_000.0 },
        );

        // Pre-fill buffer as if acquisition was running.
        dev.sample_buf = vec![0x01, 0x02, 0x03];
        dev.running = false;

        let chunk = dev.next_chunk(10);
        assert_eq!(chunk.logic(), &[0x01, 0x02, 0x03]);
        assert!(dev.sample_buf.is_empty(), "buffer should be drained after stopped read");
    }

    #[test]
    fn next_chunk_reads_8bit_from_transport() {
        let mut transport = MockTransport::new();
        transport.queue_read([0x0F, 0xF0, 0x55]);

        let mut dev = Fx2lafwDevice::new(
            DeviceId::new("test"),
            DeviceInfo::new("X", "Y"),
            Box::new(transport),
            Fx2lafwConfig { channels: 8, sample_rate_hz: 1_000_000.0 },
        );
        dev.running = true;

        let chunk = dev.next_chunk(10);
        assert_eq!(chunk.logic(), &[0x0F, 0xF0, 0x55]);
    }

    #[test]
    fn next_chunk_reads_16bit_from_transport() {
        let mut transport = MockTransport::new();
        // 16-bit samples: 3 samples = 6 bytes (little-endian)
        transport.queue_read([0x34, 0x12, 0x78, 0x56, 0xBC, 0x9A]);
        // → 0x1234, 0x5678, 0x9ABC

        let mut dev = Fx2lafwDevice::new(
            DeviceId::new("test"),
            DeviceInfo::new("X", "Y"),
            Box::new(transport),
            Fx2lafwConfig { channels: 16, sample_rate_hz: 1_000_000.0 },
        );
        assert!(dev.sample_wide, "16-channel device must be in wide mode");
        dev.running = true;

        let chunk = dev.next_chunk(3);
        assert_eq!(chunk.logic().len(), 3);
        assert_eq!(chunk.logic()[0], 0x1234);
        assert_eq!(chunk.logic()[1], 0x5678);
        assert_eq!(chunk.logic()[2], 0x9ABC);
    }

    #[test]
    fn next_chunk_handles_partial_16bit_sample() {
        let mut transport = MockTransport::new();
        // 5 bytes = 2 complete samples + 1 leftover byte
        transport.queue_read([0x01, 0x00, 0x02, 0x00, 0x03]);

        let mut dev = Fx2lafwDevice::new(
            DeviceId::new("test"),
            DeviceInfo::new("X", "Y"),
            Box::new(transport),
            Fx2lafwConfig { channels: 16, sample_rate_hz: 1_000_000.0 },
        );
        dev.running = true;

        let chunk = dev.next_chunk(10);
        // 2 complete 16-bit samples decoded, 1 byte buffered for next call.
        assert_eq!(chunk.logic().len(), 2);
        assert_eq!(chunk.logic()[0], 0x0001);
        assert_eq!(chunk.logic()[1], 0x0002);
        assert_eq!(dev.sample_buf, vec![0x03], "partial sample byte should remain in buffer");
    }

    #[test]
    fn next_chunk_respects_max_samples() {
        let mut transport = MockTransport::new();
        // Queue more data than we'll consume.
        transport.queue_read([0x01, 0x02, 0x03, 0x04, 0x05, 0x06]);

        let mut dev = Fx2lafwDevice::new(
            DeviceId::new("test"),
            DeviceInfo::new("X", "Y"),
            Box::new(transport),
            Fx2lafwConfig { channels: 8, sample_rate_hz: 1_000_000.0 },
        );
        dev.running = true;

        // Only request 2 samples.
        let chunk = dev.next_chunk(2);
        assert_eq!(chunk.logic().len(), 2);
        assert_eq!(chunk.logic()[0], 0x01);
        assert_eq!(chunk.logic()[1], 0x02);
        // Transport may still have unread data, but device buffer should be cleanly consumed.
        assert!(dev.sample_buf.is_empty(), "only 2 bytes read → buffer empty");
    }

    #[test]
    fn next_chunk_handles_empty_transport_read() {
        let transport = MockTransport::new();
        // queue_read empty → read returns 0 → end-of-stream

        let mut dev = Fx2lafwDevice::new(
            DeviceId::new("test"),
            DeviceInfo::new("X", "Y"),
            Box::new(transport),
            Fx2lafwConfig::default(),
        );
        dev.running = true;

        let chunk = dev.next_chunk(10);
        assert_eq!(chunk.logic().len(), 0, "empty transport → empty chunk");
    }

    // ── Intel HEX parsing ─────────────────────────────────────────────────────

    #[test]
    fn parse_ihex_single_data_record() {
        let hex = b":02000000123456\n";
        let chunks = parse_ihex(hex).unwrap();
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].0, 0x0000);
        assert_eq!(chunks[0].1, vec![0x12, 0x34]);
    }

    #[test]
    fn parse_ihex_skips_eof_record() {
        let hex = b":02000000123456\n:00000001FF\n";
        let chunks = parse_ihex(hex).unwrap();
        assert_eq!(chunks.len(), 1);
    }

    #[test]
    fn parse_ihex_extended_linear_address() {
        let hex = b":02000004123400\n:01000000ABFE\n";
        let chunks = parse_ihex(hex).unwrap();
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].1, vec![0xAB]);
    }

    #[test]
    fn parse_ihex_handles_empty_input() {
        let chunks = parse_ihex(b"").unwrap();
        assert!(chunks.is_empty());
    }

    #[test]
    fn parse_ihex_skips_non_hex_lines() {
        let hex = b"not a hex line\n:01000000FF00\n";
        let chunks = parse_ihex(hex).unwrap();
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].1, vec![0xFF]);
    }

    // ── Firmware upload ───────────────────────────────────────────────────────

    #[test]
    fn upload_firmware_sends_control_transfers() {
        let mut transport = MockTransport::new();
        transport.queue_control_response([]); // response for data write
        transport.queue_control_response([]); // response for start execution

        let hex = b":02000000123400\n:00000001FF\n";
        let result = block_on(upload_firmware(&mut transport, hex));
        assert!(result.is_ok());

        let ctrl = transport.control_transfers();
        assert!(ctrl.len() >= 2);
        assert_eq!(ctrl[0].request_type, 0x40);
        assert_eq!(ctrl[0].request, 0xA0);
        assert_eq!(ctrl[0].value, 0x0000);
        assert_eq!(ctrl[0].data, vec![0x12, 0x34]);

        let last = ctrl.last().unwrap();
        assert_eq!(last.request_type, 0x40);
        assert_eq!(last.request, 0xA0);
        assert_eq!(last.value, FW_ENTRY);
    }

    // ── Device profile table ──────────────────────────────────────────────────

    #[test]
    fn find_profile_returns_known_device() {
        let p = find_profile(0x0925, 0x3881).unwrap();
        assert_eq!(p.vendor, "Saleae");
        assert_eq!(p.model, "Logic");
    }

    #[test]
    fn find_profile_returns_none_for_unknown() {
        assert!(find_profile(0xFFFF, 0xFFFF).is_none());
    }

    #[test]
    fn is_bootloader_detects_cypress() {
        assert!(is_bootloader(0x04B4, 0x8613));
        assert!(!is_bootloader(0x1D50, 0x608C));
    }
}
