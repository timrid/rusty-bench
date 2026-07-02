//! **fx2lafw** — Logic Analyzer driver for FX2LP-based devices running the
//! [fx2lafw](https://sigrok.org/wiki/Fx2lafw) firmware.
//!
//! Implements the sigrok fx2lafw USB vendor-request protocol (clean-room,
//! no GPLv3 source referenced).
//!
//! # Protocol overview
//! - Firmware upload: Cypress EZ-USB bootloader via control endpoint 0 (vendor
//!   request `0xA0`).  The FX2LP firmware (`.fw`) is written to internal RAM
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
//! The driver is tested against [`MockUsbTransport`] for all protocol logic.
//!
//! [`MockUsbTransport`]: rb_transport::MockUsbTransport

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use async_trait::async_trait;
use futures::channel::{mpsc, oneshot};

use rb_device::{
    AcquisitionSource, Device, DeviceClass, DeviceError, DeviceId, DeviceInfo, DeviceResult,
    LogicAnalyzer,
};
use rb_model::{DigitalChannel, SampleChunk};
use rb_transport::{DeviceCandidate, DriverError, DriverFactory, DriverResult, UsbTransport};

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

// -- Transfer buffer sizing --------------

/// Maximum number of concurrent bulk-IN transfers.
const NUM_SIMUL_TRANSFERS: usize = 32;
/// Consecutive empty transfers before assuming the device has stalled.
const MAX_EMPTY_TRANSFERS: usize = NUM_SIMUL_TRANSFERS * 2;
/// Buffer alignment (round buffer sizes up to multiples of 512).
const BUFFER_ALIGN: usize = 512;
/// Each transfer buffer should hold this many milliseconds of sample data.
const BUFFER_DURATION_MS: f64 = 10.0;
/// Total queued buffer capacity (across all transfers) in milliseconds.
const TOTAL_BUFFER_DURATION_MS: f64 = 500.0;

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

// ── Transfer buffer sizing ─────────────────────────

/// Bytes per millisecond at the given sample rate.
fn to_bytes_per_ms(samplerate_hz: f64, sample_wide: bool) -> f64 {
    let bytes_per_sample = if sample_wide { 2.0 } else { 1.0 };
    samplerate_hz * bytes_per_sample / 1000.0
}

/// Compute a single transfer buffer size: `BUFFER_DURATION_MS` of data,
/// rounded up to the next multiple of [`BUFFER_ALIGN`].
fn compute_buffer_size(samplerate_hz: f64, sample_wide: bool) -> usize {
    let s = BUFFER_DURATION_MS * to_bytes_per_ms(samplerate_hz, sample_wide);
    // Round up to BUFFER_ALIGN (512).
    let s = (s as usize).max(1);
    (s + BUFFER_ALIGN - 1) & !(BUFFER_ALIGN - 1)
}

/// Number of concurrent transfers needed for ~500ms total buffering,
/// capped at [`NUM_SIMUL_TRANSFERS`].
fn compute_num_transfers(samplerate_hz: f64, sample_wide: bool) -> usize {
    let n = (TOTAL_BUFFER_DURATION_MS * to_bytes_per_ms(samplerate_hz, sample_wide)
        / compute_buffer_size(samplerate_hz, sample_wide) as f64) as usize;
    n.clamp(1, NUM_SIMUL_TRANSFERS)
}

// ── Device ─────────────────────────────────────────────────────────────────────

/// An fx2lafw-based logic analyzer device.
///
/// All protocol I/O goes through the owned [`UsbTransport`], making the device
/// testable via [`MockUsbTransport`](rb_transport::MockUsbTransport).
///
/// The `transport` is wrapped in `Option` so [`start_streaming`] can move it
/// into the read-loop future.  After `start_streaming`, `transport` is `None`
/// until the read loop ends.
///
/// [`start_streaming`]: AcquisitionSource::start_streaming
pub struct Fx2lafwDevice {
    id: DeviceId,
    info: DeviceInfo,
    transport: Option<Box<dyn UsbTransport>>,
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
    /// Whether acquisition has been started (shared with read-loop future).
    running: Arc<AtomicBool>,
    /// Pre-computed arm state: start-acquisition flags (set by `arm()`).
    arm_flags: u8,
    /// Pre-computed arm state: GPIF delay (set by `arm()`).
    arm_delay: u16,
    /// Receiver for the transport returned by a completed read-loop.
    /// Populated by [`start_streaming`]; awaited on re-arm.
    pending_transport: Option<oneshot::Receiver<Box<dyn UsbTransport>>>,
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
        transport: Box<dyn UsbTransport>,
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
            transport: Some(transport),
            config: Fx2lafwConfig {
                channels: ch,
                sample_rate_hz: config.sample_rate_hz,
            },
            fw_version: (0, 0),
            sample_wide,
            clock_hz: CLOCK_48MHZ,
            actual_rate: 0.0,
            channels,
            running: Arc::new(AtomicBool::new(false)),
            arm_flags: 0,
            arm_delay: 0,
            pending_transport: None,
        }
    }

    /// Borrows the transport, panicking if it has been moved out (e.g. by an
    /// active read loop).
    fn transport_mut(&mut self) -> &mut dyn UsbTransport {
        self.transport
            .as_mut()
            .expect("transport moved out — acquisition already running")
            .as_mut()
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
        self.fw_version = get_fw_version(self.transport_mut()).await?;
        if self.fw_version.0 != REQUIRED_FW_MAJOR {
            return Err(DeviceError::Protocol(format!(
                "unsupported firmware version {}.{} (need {}.x)",
                self.fw_version.0, self.fw_version.1, REQUIRED_FW_MAJOR
            )));
        }

        // Query chip revision.
        let revid = get_revid(self.transport_mut()).await?;
        let chip = if revid == 1 {
            "FX2LP (CY7C68013A)"
        } else {
            "FX2 (CY7C68013)"
        };

        // Update device info with discovered details.
        let vendor = core::mem::take(&mut self.info.vendor);
        let model = core::mem::take(&mut self.info.model);
        let serial = self.info.serial.take();
        let mut info = DeviceInfo::new(
            vendor,
            format!(
                "{model} fw:{}.{} chip:{chip}",
                self.fw_version.0, self.fw_version.1
            ),
        );
        info.serial = serial;
        self.info = info;

        Ok(())
    }

    async fn close(&mut self) -> DeviceResult<()> {
        if let Some(ref mut transport) = self.transport {
            let _ = transport.close().await;
        }
        Ok(())
    }
}

// ── Command helpers ────────────────────────────────────────────────────────────

/// Query firmware version from the device.
///
/// Sends `CMD_GET_FW_VERSION` (0xB0) as a vendor IN request and reads the
/// 2-byte `{major, minor}` response.
async fn get_fw_version(transport: &mut dyn UsbTransport) -> DeviceResult<(u8, u8)> {
    let resp = transport
        .control_transfer(
            0xC0,               // vendor, device-to-host (IN)
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
async fn get_revid(transport: &mut dyn UsbTransport) -> DeviceResult<u8> {
    let resp = transport
        .control_transfer(
            0xC0,          // vendor, device-to-host (IN)
            CMD_GET_REVID, // bRequest = 0xB2
            0,             // wValue
            0,             // wIndex
            &[],           // no OUT data
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
    transport: &mut dyn UsbTransport,
    flags: u8,
    delay: u16,
) -> DeviceResult<()> {
    let delay_h = (delay >> 8) as u8;
    let delay_l = (delay & 0xFF) as u8;
    let data = [flags, delay_h, delay_l];
    transport
        .control_transfer(
            0x40,          // vendor, host-to-device (OUT)
            CMD_START_ACQ, // bRequest = 0xB1
            0,             // wValue
            0,             // wIndex
            &data,         // 3-byte data phase
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
        // Reset the bulk-IN pipe before starting acquisition.  After a fresh
        // plug-in, the first WinUsb_ReadPipe after CMD_START_ACQ works, but
        // the second may hang forever.  WinUsb_ResetPipe clears this condition.
        // On re-arm the transport is still in-flight via the oneshot return
        // channel; skip the halt clear in that case.
        if let Some(transport) = self.transport.as_mut() {
            transport
                .clear_in_halt()
                .await
                .map_err(|e| DeviceError::Transport(format!("clear_in_halt before arm: {e}")))?;
        }

        // Compute delay and flags now, but defer the actual GPIF start
        // to `start_streaming()`.  This lets `start_streaming()` queue all
        // USB bulk-IN transfers BEFORE starting the GPIF engine — otherwise
        // the FX2's tiny EP2 buffer overflows immediately.
        let dc = compute_delay(self.config.sample_rate_hz);
        let clock_48mhz = (dc.clock_hz - CLOCK_48MHZ).abs() < 1.0;
        self.arm_flags = start_flags(self.sample_wide, clock_48mhz, false);
        self.arm_delay = dc.delay;
        self.clock_hz = dc.clock_hz;
        self.actual_rate = dc.actual_rate_hz;

        Ok(())
    }

    async fn stop(&mut self) -> DeviceResult<()> {
        // The fx2lafw firmware has no explicit stop command; the GPIF engine
        // stops when its waveform completes or the host stops reading.
        self.running.store(false, Ordering::SeqCst);
        Ok(())
    }
}

#[async_trait(?Send)]
impl AcquisitionSource for Fx2lafwDevice {
    async fn start_streaming(
        &mut self,
        chunk_tx: mpsc::UnboundedSender<SampleChunk>,
    ) -> DeviceResult<Pin<Box<dyn Future<Output = ()>>>> {
        // Take ownership of transport for the read loop.
        // On re-arm after stop, wait for the previous read-loop to return it.
        let mut transport = match self.transport.take() {
            Some(t) => t,
            None => {
                self.pending_transport
                    .take()
                    .ok_or_else(|| {
                        DeviceError::Protocol(
                            "transport not available (acquisition already running?)".into(),
                        )
                    })?
                    .await
                    .map_err(|_| DeviceError::Protocol("transport channel closed".into()))?
            }
        };
        let sample_wide = self.sample_wide;
        let running = self.running.clone();
        let arm_flags = self.arm_flags;
        let arm_delay = self.arm_delay;
        let sample_rate_hz = self.actual_rate;

        // ── Compute adaptive buffer sizing ────────────────
        let buffer_size = compute_buffer_size(sample_rate_hz, sample_wide);
        let num_transfers = compute_num_transfers(sample_rate_hz, sample_wide);
        log::debug!(
            "fx2lafw: adaptive buffer: {} transfers × {} bytes (rate={:.0} Hz, wide={})",
            num_transfers, buffer_size, sample_rate_hz, sample_wide
        );

        // Channel to return the transport when the read-loop exits (re-arm support).
        let (return_tx, return_rx) = oneshot::channel();
        self.pending_transport = Some(return_rx);

        let fut = async move {
            let mut buf = Vec::new();
            let mut empty_transfer_count: usize = 0;

            // ── Submit transfers FIRST, BEFORE starting the GPIF engine ────
            // This is the critical ordering fix: by the time the FX2 starts
            // producing data, USB transfers are already queued to receive it.
            for _ in 0..num_transfers {
                transport.submit_bulk_in(vec![0u8; buffer_size]);
            }

            // ── Now start the GPIF engine ──────────────────────────────────
            if let Err(e) = start_acquisition(&mut *transport, arm_flags, arm_delay).await {
                log::error!("fx2lafw: start_acquisition failed: {e}");
                running.store(false, Ordering::SeqCst);
                return;
            }
            running.store(true, Ordering::SeqCst);

            loop {
                if !running.load(Ordering::SeqCst) {
                    // Final drain: send remaining buffered samples.
                    let sample_bytes = if sample_wide { 2 } else { 1 };
                    let complete = (buf.len() / sample_bytes) * sample_bytes;
                    if complete > 0 {
                        let chunk =
                            decode_samples(&buf[..complete], sample_wide, complete / sample_bytes);
                        let _ = chunk_tx.unbounded_send(chunk);
                    }
                    break;
                }

                // Read from USB EP2 IN — suspends until data arrives.
                match transport.next_bulk_in().await {
                    Ok(data) if data.is_empty() => {
                        empty_transfer_count += 1;
                        if empty_transfer_count > MAX_EMPTY_TRANSFERS {
                            log::warn!(
                                "fx2lafw: {} consecutive empty transfers — \
                                 device stalled (buffer overflow?)",
                                empty_transfer_count
                            );
                            running.store(false, Ordering::SeqCst);
                        } else {
                            // Re-submit the (empty) buffer and keep waiting.
                            transport.submit_bulk_in(data);
                        }
                    }
                    Err(e) => {
                        log::warn!("fx2lafw: USB read error: {e}");
                        running.store(false, Ordering::SeqCst);
                    }
                    Ok(data) => {
                        empty_transfer_count = 0;
                        buf.extend_from_slice(&data);
                        // Re-submit the SAME buffer (buffer reuse).
                        // This reuses the buffer that just completed instead
                        // of allocating a new `Vec<u8>` on every transfer.
                        transport.submit_bulk_in(data);

                        // Decode as many complete samples as possible and send.
                        let sample_bytes = if sample_wide { 2 } else { 1 };
                        let complete = (buf.len() / sample_bytes) * sample_bytes;
                        if complete >= sample_bytes {
                            let max = complete / sample_bytes;
                            let chunk = decode_samples(&buf[..complete], sample_wide, max);
                            let consumed = chunk.logic().len() * sample_bytes;
                            buf.drain(..consumed);

                            if !chunk.is_empty() {
                                if chunk_tx.unbounded_send(chunk).is_err() {
                                    break; // receiver dropped
                                }
                            }
                        }
                    }
                }
            }

            running.store(false, Ordering::SeqCst);
            // Cancel any remaining pending bulk-IN transfers so the pipe is
            // clean for the next acquisition (re-arm support).
            let _ = transport.clear_in_halt().await;
            // Return the transport so it can be reused on re-arm.
            let _ = return_tx.send(transport);
        };

        Ok(Box::pin(fut))
    }

    async fn stop_streaming(&mut self) -> DeviceResult<()> {
        self.running.store(false, Ordering::SeqCst);
        Ok(())
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

/// Upload raw binary firmware (sigrok `.fw` format) to the FX2LP via the
/// Cypress EZ-USB vendor request protocol:
///
/// 1. Put CPU in reset (write `0x01` to CPUCS register at `0xE600`).
/// 2. Write firmware data in 4 KB chunks starting at address `0x0000`.
/// 3. Release CPU reset (write `0x00` to CPUCS), which starts execution.
///
/// # Errors
/// Returns an error if any control transfer fails.
pub async fn upload_firmware(
    transport: &mut dyn UsbTransport,
    data: &[u8],
) -> Result<(), String> {
    const MAX_CHUNK: usize = 4096;
    /// CPU Control & Status register (FX2LP internal address).
    const CPUCS_ADDR: u16 = 0xE600;

    // 1. Put CPU in reset.
    transport
        .control_transfer(0x40, CYPRESS_FW_WRITE, CPUCS_ADDR, 0x0000, &[0x01])
        .await
        .map_err(|e| format!("CPU reset on failed: {e}"))?;

    // 2. Upload firmware in 4 KB chunks.
    for (offset, chunk) in data.chunks(MAX_CHUNK).enumerate() {
        let addr = (offset * MAX_CHUNK) as u16;
        transport
            .control_transfer(0x40, CYPRESS_FW_WRITE, addr, 0x0000, chunk)
            .await
            .map_err(|e| format!("firmware upload failed at 0x{addr:04X}: {e}"))?;
    }

    // 3. Release CPU reset → device resets and re-enumerates.
    // The control transfer often fails with "device disconnected" because
    // the FX2LP resets before the USB stack can send the ACK. This is
    // expected — we just log the outcome and proceed to wait for
    // renumeration.
    let _ = transport
        .control_transfer(0x40, CYPRESS_FW_WRITE, CPUCS_ADDR, 0x0000, &[0x00])
        .await;
    // Ignore errors here — the device is already resetting.

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
    /// Firmware file name (e.g. `"fx2lafw-saleae-logic.fw"`).
    /// `None` for devices that ship with fx2lafw firmware pre-installed
    /// (sigrok VID/PID).
    firmware_file: Option<&'static str>,
}

/// Known fx2lafw-compatible devices.
static SUPPORTED_DEVICES: &[DeviceProfile] = &[
    // Cypress FX2 (no EEPROM) — bootloader VID/PID
    DeviceProfile {
        vid: 0x04B4,
        pid: 0x8613,
        vendor: "Cypress",
        model: "FX2 (bootloader)",
        has_16bit: true,
        firmware_file: Some("fx2lafw-cypress-fx2.fw"),
    },
    // USBee AX (and clones)
    DeviceProfile {
        vid: 0x08A9,
        pid: 0x0014,
        vendor: "CWAV",
        model: "USBee AX",
        has_16bit: false,
        firmware_file: Some("fx2lafw-cwav-usbeeax.fw"),
    },
    // USBee DX
    DeviceProfile {
        vid: 0x08A9,
        pid: 0x0015,
        vendor: "CWAV",
        model: "USBee DX",
        has_16bit: true,
        firmware_file: Some("fx2lafw-cwav-usbeedx.fw"),
    },
    // USBee SX
    DeviceProfile {
        vid: 0x08A9,
        pid: 0x0009,
        vendor: "CWAV",
        model: "USBee SX",
        has_16bit: false,
        firmware_file: Some("fx2lafw-cwav-usbeesx.fw"),
    },
    // USBee ZX
    DeviceProfile {
        vid: 0x08A9,
        pid: 0x0005,
        vendor: "CWAV",
        model: "USBee ZX",
        has_16bit: false,
        firmware_file: Some("fx2lafw-cwav-usbeezx.fw"),
    },
    // Saleae Logic (and many clones)
    DeviceProfile {
        vid: 0x0925,
        pid: 0x3881,
        vendor: "Saleae",
        model: "Logic",
        has_16bit: false,
        firmware_file: Some("fx2lafw-saleae-logic.fw"),
    },
    // Braintechnology USB-LPS
    DeviceProfile {
        vid: 0x16D0,
        pid: 0x0498,
        vendor: "Braintechnology",
        model: "USB-LPS",
        has_16bit: true,
        firmware_file: Some("fx2lafw-braintechnology-usb-lps.fw"),
    },
    // sigrok FX2 LA 8ch
    DeviceProfile {
        vid: 0x1D50,
        pid: 0x608C,
        vendor: "sigrok",
        model: "FX2 LA (8ch)",
        has_16bit: false,
        firmware_file: Some("fx2lafw-sigrok-fx2-8ch.fw"),
    },
    // sigrok FX2 LA 16ch
    DeviceProfile {
        vid: 0x1D50,
        pid: 0x608D,
        vendor: "sigrok",
        model: "FX2 LA (16ch)",
        has_16bit: true,
        firmware_file: Some("fx2lafw-sigrok-fx2-16ch.fw"),
    },
    // fx2lafw-generic PIDs (pre-installed firmware, no upload needed)
    DeviceProfile {
        vid: 0x1D50,
        pid: 0x6081,
        vendor: "sigrok",
        model: "fx2lafw",
        has_16bit: false,
        firmware_file: None,
    },
    DeviceProfile {
        vid: 0x1D50,
        pid: 0x6082,
        vendor: "sigrok",
        model: "fx2lafw",
        has_16bit: false,
        firmware_file: None,
    },
    DeviceProfile {
        vid: 0x1D50,
        pid: 0x608E,
        vendor: "sigrok",
        model: "fx2lafw",
        has_16bit: false,
        firmware_file: None,
    },
    // sigrok usb-c-grok
    DeviceProfile {
        vid: 0x1D50,
        pid: 0x608F,
        vendor: "sigrok",
        model: "usb-c-grok",
        has_16bit: false,
        firmware_file: Some("fx2lafw-usb-c-grok.fw"),
    },
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

/// Returns the list of known fx2lafw-compatible VID/PID pairs.
/// Useful for building WebUSB `requestDevice()` filters.
#[must_use]
pub fn known_vid_pids() -> Vec<(u16, u16)> {
    SUPPORTED_DEVICES
        .iter()
        .map(|p| (p.vid, p.pid))
        .collect()
}

// ── Driver factory ────────────────────────────────────────────────────────────

/// Driver factory for fx2lafw-based logic analyzers.
///
/// On native builds (with the `fx2lafw` feature + `nusb`) the factory
/// enumerates USB devices directly.  On the web the caller must provide
/// pre-authorized candidates via [`set_candidates`](Self::set_candidates).
///
/// To enable automatic firmware upload for bootloader devices (Cypress FX2
/// without EEPROM), set a [`crate::FirmwareLoader`] via
/// [`set_firmware_loader`](Self::set_firmware_loader).
pub struct Fx2lafwFactory {
    candidates: Vec<DeviceCandidate>,
    firmware_loader: Option<Box<dyn crate::FirmwareLoader>>,
}

impl Fx2lafwFactory {
    /// Creates a factory with an empty candidate list and no firmware loader.
    #[must_use]
    pub fn new() -> Self {
        Self {
            candidates: Vec::new(),
            firmware_loader: None,
        }
    }

    /// Sets (or replaces) the list of pre-enumerated candidates (web use).
    pub fn set_candidates(&mut self, candidates: Vec<DeviceCandidate>) {
        self.candidates = candidates;
    }

    /// Sets the firmware loader used to upload firmware to bootloader devices
    /// during [`scan`](DriverFactory::scan).
    pub fn set_firmware_loader(
        &mut self,
        loader: Box<dyn crate::FirmwareLoader>,
    ) {
        self.firmware_loader = Some(loader);
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
        connect_usb(candidate, self.firmware_loader.as_deref()).await
    }
}

/// Enumerate USB devices matching known fx2lafw or Cypress bootloader VID/PID pairs.
///
/// Bootloader devices are included in the results with a boot flag in the
/// address string.  Firmware upload happens during [`connect`], not here.
#[cfg(feature = "fx2lafw")]
async fn scan_usb() -> DriverResult<Vec<DeviceCandidate>> {
    let devices = nusb::list_devices()
        .await
        .map_err(|e| DriverError::Transport(rb_transport::TransportError::Io(e.to_string())))?;

    let mut results = Vec::new();
    for dev in devices {
        let vid = dev.vendor_id();
        let pid = dev.product_id();
        let Some(profile) = find_profile(vid, pid) else {
            continue;
        };

        let serial = dev.serial_number().unwrap_or("").to_string();
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
/// If the device is not yet running fx2lafw firmware but a
/// [`crate::FirmwareLoader`] is available, firmware is uploaded
/// automatically.  The Cypress FX2LP hardware handles the 0xA0 vendor
/// request regardless of which firmware is currently loaded, so this
/// works for bootloader devices (0x04B4:0x8613) as well as devices
/// running original vendor firmware (e.g. Saleae Logic at 0x0925:0x3881).
#[cfg(feature = "fx2lafw")]
async fn connect_usb(
    candidate: &DeviceCandidate,
    firmware_loader: Option<&dyn crate::FirmwareLoader>,
) -> DriverResult<Box<dyn Device>> {
    // Parse the address: "VID:PID:SERIAL:BOOTFLAG"
    let parts: Vec<&str> = candidate.address.split(':').collect();
    if parts.len() < 2 {
        return Err(DriverError::NotFound);
    }
    let target_vid = u16::from_str_radix(parts[0], 16).map_err(|_| DriverError::NotFound)?;
    let target_pid = u16::from_str_radix(parts[1], 16).map_err(|_| DriverError::NotFound)?;
    let target_serial = parts.get(2).copied().unwrap_or("");

    let profile =
        find_profile(target_vid, target_pid).ok_or(DriverError::NotFound)?;

    // ── Firmware upload: needed if device doesn't already run fx2lafw ──
    if let (Some(loader), Some(fw_name)) =
        (firmware_loader, profile.firmware_file)
    {
        // Quick check: is fx2lafw already running?
        let mut needs_upload = true;
        if let Ok(dev_info) =
            find_usb_device(target_vid, target_pid, target_serial).await
        {
            if let Ok(device) = dev_info.open().await {
                if let Ok(interface) = device.detach_and_claim_interface(0).await
                {
                    let mut transport: Box<dyn UsbTransport> = Box::new(
                        rb_transport::nusb::NusbTransport::new(
                            interface, 0x00, 0x00,
                        )
                        .map_err(|e| {
                            DriverError::Transport(
                                rb_transport::TransportError::Io(e.to_string()),
                            )
                        })?,
                    );
                    // Try the fx2lafw-specific vendor request.
                    // Any error means the device is not running fx2lafw.
                    if let Ok((major, _minor)) =
                        get_fw_version(&mut *transport).await
                    {
                        if major == REQUIRED_FW_MAJOR {
                            log::debug!(
                                "fx2lafw: {:04X}:{:04X} already running fx2lafw v{major}",
                                target_vid, target_pid
                            );
                            needs_upload = false;
                        }
                    }
                    drop(transport);
                }
            }
        }

        if needs_upload {
            log::info!(
                "fx2lafw: {:04X}:{:04X} needs firmware, uploading {} …",
                target_vid, target_pid, fw_name
            );

            // 1. Load firmware bytes.
            let fw_data = loader.load_firmware(fw_name).await.map_err(|e| {
                DriverError::Transport(rb_transport::TransportError::Io(e))
            })?;

            // 2. Open the device.
            let dev_info =
                find_usb_device(target_vid, target_pid, target_serial)
                    .await?;
            let device = dev_info.open().await.map_err(|e| {
                DriverError::Transport(rb_transport::TransportError::Io(
                    e.to_string(),
                ))
            })?;
            let interface =
                device.detach_and_claim_interface(0).await.map_err(|e| {
                    DriverError::Transport(rb_transport::TransportError::Io(
                        e.to_string(),
                    ))
                })?;

            let mut transport: Box<dyn UsbTransport> = Box::new(
                rb_transport::nusb::NusbTransport::new(interface, 0x00, 0x00)
                    .map_err(|e| {
                        DriverError::Transport(
                            rb_transport::TransportError::Io(e.to_string()),
                        )
                    })?,
            );

            // 3. Upload firmware via Cypress 0xA0 vendor request.
            upload_firmware(&mut *transport, &fw_data).await.map_err(
                |e| {
                    DriverError::Transport(rb_transport::TransportError::Io(
                        e,
                    ))
                },
            )?;
            drop(transport);

            // 4. Wait for renumeration.
            let max_polls =
                (MAX_RENUM_DELAY_MS / RENUM_POLL_MS) as usize;
            for _ in 0..max_polls {
                futures_timer::Delay::new(std::time::Duration::from_millis(
                    RENUM_POLL_MS,
                ))
                .await;

                let devices = nusb::list_devices().await.map_err(|e| {
                    DriverError::Transport(
                        rb_transport::TransportError::Io(e.to_string()),
                    )
                })?;

                for dev in devices {
                    let dev_vid = dev.vendor_id();
                    let dev_pid = dev.product_id();
                    if let Some(re_profile) = find_profile(dev_vid, dev_pid) {
                        let dev_serial =
                            dev.serial_number().unwrap_or("");
                        // Reuse the original candidate address so the
                        // connected device matches the scan result in the UI.
                        let address = candidate.address.clone();
                        let mut info = DeviceInfo::new(
                            re_profile.vendor,
                            re_profile.model,
                        );
                        if !dev_serial.is_empty() {
                            info = info.with_serial(dev_serial);
                        }

                        log::info!(
                            "fx2lafw: re-enumerated as {:04X}:{:04X}",
                            dev_vid, dev_pid
                        );

                        let new_candidate =
                            DeviceCandidate::new(info, address);
                        return open_and_connect(
                            &new_candidate,
                            dev_vid,
                            dev_pid,
                            dev_serial,
                        )
                        .await;
                    }
                }
            }

            return Err(DriverError::Transport(
                rb_transport::TransportError::Io(
                    "device did not re-enumerate after firmware upload"
                        .into(),
                ),
            ));
        }
    }

    // ── Normal device (already has fx2lafw firmware) ───────────────────────

    open_and_connect(candidate, target_vid, target_pid, target_serial).await
}

/// Open a USB device by VID/PID/serial and wrap it in an [`Fx2lafwDevice`].
#[cfg(feature = "fx2lafw")]
async fn open_and_connect(
    candidate: &DeviceCandidate,
    vid: u16,
    pid: u16,
    serial: &str,
) -> DriverResult<Box<dyn Device>> {
    let dev_info = find_usb_device(vid, pid, serial).await?;
    let device = dev_info
        .open()
        .await
        .map_err(|e| DriverError::Transport(rb_transport::TransportError::Io(e.to_string())))?;

    // Claim interface 0 (the only interface fx2lafw exposes).
    let interface = device
        .detach_and_claim_interface(0)
        .await
        .map_err(|e| DriverError::Transport(rb_transport::TransportError::Io(e.to_string())))?;

    // fx2lafw firmware only uses EP2 IN (0x82) for sample data.
    // All commands go through EP0 control transfers — no bulk OUT endpoint needed.
    let transport: Box<dyn UsbTransport> = Box::new(rb_transport::nusb::NusbTransport::new(
        interface, EP_DATA, // bulk IN  = EP2 IN  (0x82)
        0x00,    // bulk OUT = unused
    ).map_err(|e| DriverError::Transport(rb_transport::TransportError::Io(e.to_string())))?);

    let id = DeviceId::new(&candidate.address);
    let info = candidate.info.clone();
    let config = Fx2lafwConfig::default();

    Ok(Box::new(Fx2lafwDevice::new(id, info, transport, config)))
}

/// Upload firmware to a device in bootloader mode, then wait for renumeration
/// and return a connected [`Device`].
///
/// The `firmware_data` is raw binary (sigrok `.fw` format).
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
        .await
        .map_err(|e| DriverError::Transport(rb_transport::TransportError::Io(e.to_string())))?;

    let interface = boot_device
        .detach_and_claim_interface(0)
        .await
        .map_err(|e| DriverError::Transport(rb_transport::TransportError::Io(e.to_string())))?;

    let mut transport: Box<dyn UsbTransport> = Box::new(rb_transport::nusb::NusbTransport::new(
        interface, 0x00, 0x00,
    ).map_err(|e| DriverError::Transport(rb_transport::TransportError::Io(e.to_string())))?);

    // 2. Upload firmware.
    upload_firmware(&mut *transport, firmware_data)
        .await
        .map_err(|e| DriverError::Transport(rb_transport::TransportError::Io(e)))?;

    // The device disconnects and re-enumerates with new VID/PID.
    drop(transport);

    // 3. Wait for renumeration — poll until a fx2lafw device appears.
    let max_polls = (MAX_RENUM_DELAY_MS / RENUM_POLL_MS) as usize;
    for _ in 0..max_polls {
        futures_timer::Delay::new(std::time::Duration::from_millis(RENUM_POLL_MS)).await;

        let devices = nusb::list_devices()
            .await
            .map_err(|e| DriverError::Transport(rb_transport::TransportError::Io(e.to_string())))?;

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
                    return connect_usb(&candidate, None).await;
                }
            }
        }
    }

    Err(DriverError::Transport(rb_transport::TransportError::Io(
        "device did not re-enumerate after firmware upload".into(),
    )))
}

/// Find a USB device by VID, PID, and optional serial.
#[cfg(feature = "fx2lafw")]
async fn find_usb_device(vid: u16, pid: u16, serial: &str) -> DriverResult<nusb::DeviceInfo> {
    nusb::list_devices()
        .await
        .map_err(|e| DriverError::Transport(rb_transport::TransportError::Io(e.to_string())))?
        .find(|dev| {
            dev.vendor_id() == vid
                && dev.product_id() == pid
                && (serial.is_empty() || dev.serial_number().unwrap_or("") == serial)
        })
        .ok_or(DriverError::NotFound)
}

/// Returns the list of firmware file names for all supported devices that
/// require a firmware upload (i.e. devices that ship without fx2lafw
/// pre-installed).
#[must_use]
pub fn known_firmware_files() -> Vec<&'static str> {
    SUPPORTED_DEVICES
        .iter()
        .filter_map(|p| p.firmware_file)
        .collect()
}

#[cfg(not(feature = "fx2lafw"))]
async fn scan_usb() -> DriverResult<Vec<DeviceCandidate>> {
    Ok(Vec::new())
}

#[cfg(not(feature = "fx2lafw"))]
async fn connect_usb(
    _candidate: &DeviceCandidate,
    _firmware_loader: Option<&dyn crate::FirmwareLoader>,
) -> DriverResult<Box<dyn Device>> {
    Err(DriverError::NotFound)
}

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use futures::StreamExt;
    use futures::executor::block_on;
    use futures::task::LocalSpawnExt;
    use rb_transport::MockUsbTransport;

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

    // ── Device lifecycle (MockUsbTransport) ───────────────────────────────────

    #[test]
    fn device_info_and_channels_are_correct() {
        let id = DeviceId::new("fx2lafw-0");
        let info = DeviceInfo::new("Cypress", "FX2LP");
        let dev = Fx2lafwDevice::new(
            id,
            info,
            Box::new(MockUsbTransport::new()),
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
            Box::new(MockUsbTransport::new()),
            Fx2lafwConfig {
                channels: 16,
                sample_rate_hz: 1_000_000.0,
            },
        );

        assert_eq!(dev.channels().len(), 16);
        assert_eq!(dev.channels()[15].name, "D15");
        assert!(dev.sample_wide);
    }

    #[test]
    fn open_validates_fw_version() {
        let mut transport = MockUsbTransport::new();
        // Queue responses for get_fw_version (major=1, minor=4) and get_revid.
        transport.queue_control_response([1, 4]); // fw version
        transport.queue_control_response([1]); // revid (FX2LP)

        let id = DeviceId::new("test");
        let info = DeviceInfo::new("Test", "FX2");
        let mut dev = Fx2lafwDevice::new(id, info, Box::new(transport), Fx2lafwConfig::default());

        let result = block_on(dev.open());
        assert!(result.is_ok());
        assert_eq!(dev.fw_version, (1, 4));
    }

    #[test]
    fn open_rejects_wrong_fw_major() {
        let mut transport = MockUsbTransport::new();
        transport.queue_control_response([2, 0]); // fw major=2 (unsupported)
        transport.queue_control_response([1]); // revid

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
    fn arm_computes_config_but_does_not_start_acquisition() {
        let mut transport = MockUsbTransport::new();
        transport.queue_control_response([1, 4]); // fw version
        transport.queue_control_response([1]); // revid
        // NOTE: no start_acquisition response queued — arm() does not send it.

        let mut dev = Fx2lafwDevice::new(
            DeviceId::new("test"),
            DeviceInfo::new("Test", "FX2"),
            Box::new(transport),
            Fx2lafwConfig {
                channels: 8,
                sample_rate_hz: 1_000_000.0,
            },
        );

        block_on(dev.open()).unwrap();
        block_on(dev.arm()).unwrap();

        // arm() stores the computed config but does NOT start the GPIF yet.
        // running is still false (set later in start_streaming).
        assert!(!dev.running.load(Ordering::SeqCst));
        // The delay should be computed for 1 MHz: 48-1=47, actual=1MHz, clock=48MHz
        assert!((dev.actual_rate - 1_000_000.0).abs() < 1.0);
        assert!((dev.clock_hz - CLOCK_48MHZ).abs() < 1.0);
    }

    #[test]
    fn arm_stores_flags_for_48mhz_clock() {
        let mut transport = MockUsbTransport::new();
        transport.queue_control_response([1, 4]);
        transport.queue_control_response([1]);

        let mut dev = Fx2lafwDevice::new(
            DeviceId::new("test"),
            DeviceInfo::new("Test", "FX2"),
            Box::new(transport),
            Fx2lafwConfig {
                channels: 8,
                sample_rate_hz: 48_000_000.0,
            },
        );

        block_on(dev.open()).unwrap();
        block_on(dev.arm()).unwrap();
        assert!(!dev.running.load(Ordering::SeqCst));
        // At 48 MHz, delay should be 0, flags should be 8-bit + 48 MHz clock.
        assert_eq!(dev.arm_delay, 0);
        assert_ne!(dev.arm_flags & FLAG_CLK_48MHZ, 0);
    }

    // ── Direct protocol tests (MockUsbTransport, no device wrapper) ──────────

    #[test]
    fn get_fw_version_sends_correct_control_transfer() {
        let mut transport = MockUsbTransport::new();
        transport.queue_control_response([1, 4]); // {major=1, minor=4}

        let result = block_on(get_fw_version(&mut transport));
        assert_eq!(result.unwrap(), (1, 4));

        let ctrl = transport.control_transfers();
        assert_eq!(ctrl.len(), 1);
        assert_eq!(
            ctrl[0].request_type, 0xC0,
            "must be vendor IN (device→host)"
        );
        assert_eq!(
            ctrl[0].request, 0xB0,
            "bRequest must be CMD_GET_FW_VERSION (0xB0)"
        );
        assert_eq!(ctrl[0].value, 0);
        assert_eq!(ctrl[0].index, 0);
    }

    #[test]
    fn get_fw_version_rejects_short_response() {
        let mut transport = MockUsbTransport::new();
        transport.queue_control_response([0x01]); // only 1 byte, need 2

        let result = block_on(get_fw_version(&mut transport));
        assert!(result.is_err());
    }

    #[test]
    fn get_revid_sends_correct_control_transfer() {
        let mut transport = MockUsbTransport::new();
        transport.queue_control_response([0x01]); // REVID=1 → FX2LP

        let result = block_on(get_revid(&mut transport));
        assert_eq!(result.unwrap(), 1);

        let ctrl = transport.control_transfers();
        assert_eq!(ctrl.len(), 1);
        assert_eq!(
            ctrl[0].request_type, 0xC0,
            "must be vendor IN (device→host)"
        );
        assert_eq!(
            ctrl[0].request, 0xB2,
            "bRequest must be CMD_GET_REVID (0xB2)"
        );
    }

    #[test]
    fn get_revid_rejects_empty_response() {
        let mut transport = MockUsbTransport::new();
        transport.queue_control_response([]);

        let result = block_on(get_revid(&mut transport));
        assert!(result.is_err());
    }

    #[test]
    fn start_acquisition_sends_correct_control_transfer() {
        let mut transport = MockUsbTransport::new();
        transport.queue_control_response([]);

        // flags=0x21 (8-bit, 48 MHz), delay=47 (big-endian: 0x002F)
        let result = block_on(start_acquisition(&mut transport, 0x21, 47));
        assert!(result.is_ok());

        let ctrl = transport.control_transfers();
        assert_eq!(ctrl.len(), 1);
        assert_eq!(
            ctrl[0].request_type, 0x40,
            "must be vendor OUT (host→device)"
        );
        assert_eq!(
            ctrl[0].request, 0xB1,
            "bRequest must be CMD_START_ACQ (0xB1)"
        );
        assert_eq!(ctrl[0].value, 0);
        assert_eq!(ctrl[0].index, 0);
        // 3-byte data phase: [flags, delay_h, delay_l]
        assert_eq!(ctrl[0].data, vec![0x21, 0x00, 0x2F]);
    }

    #[test]
    fn start_acquisition_encodes_delay_big_endian() {
        let mut transport = MockUsbTransport::new();
        transport.queue_control_response([]);

        // delay=0x1234 → high=0x12, low=0x34
        block_on(start_acquisition(&mut transport, 0x00, 0x1234)).unwrap();

        let ctrl = transport.control_transfers();
        assert_eq!(ctrl[0].data, vec![0x00, 0x12, 0x34]);
    }

    #[test]
    fn start_acquisition_delay_zero_is_correct() {
        let mut transport = MockUsbTransport::new();
        transport.queue_control_response([]);

        // delay=0 → both bytes zero
        block_on(start_acquisition(&mut transport, 0x40, 0)).unwrap();

        let ctrl = transport.control_transfers();
        assert_eq!(ctrl[0].data, vec![0x40, 0x00, 0x00]);
    }

    #[test]
    fn start_acquisition_max_delay_encodes_correctly() {
        let mut transport = MockUsbTransport::new();
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
        let transport = MockUsbTransport::new();
        let mut dev = Fx2lafwDevice::new(
            DeviceId::new("test"),
            DeviceInfo::new("Test", "FX2"),
            Box::new(transport),
            Fx2lafwConfig::default(),
        );
        dev.running.store(true, Ordering::SeqCst); // simulate armed state

        let result = block_on(dev.stop());
        assert!(result.is_ok());
        assert!(!dev.running.load(Ordering::SeqCst));
        // No control transfers were issued (transport has no queued responses consumed).
    }

    #[test]
    fn set_sample_rate_hz_is_lazy_no_usb_command() {
        let transport = MockUsbTransport::new();
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
        let mut transport = MockUsbTransport::new();
        transport.queue_control_response([1, 4]); // fw 1.4
        transport.queue_control_response([1]); // revid=1 → FX2LP

        let mut dev = Fx2lafwDevice::new(
            DeviceId::new("test"),
            DeviceInfo::new("TestVendor", "TestModel"),
            Box::new(transport),
            Fx2lafwConfig::default(),
        );

        block_on(dev.open()).unwrap();
        assert_eq!(dev.info().vendor, "TestVendor");
        assert!(
            dev.info().model.contains("TestModel"),
            "model should contain original name"
        );
        assert!(
            dev.info().model.contains("fw:1.4"),
            "model should contain fw version"
        );
        assert!(
            dev.info().model.contains("FX2LP"),
            "model should contain chip type"
        );
    }

    #[test]
    fn open_with_revid_zero_detects_fx2() {
        let mut transport = MockUsbTransport::new();
        transport.queue_control_response([1, 2]); // fw 1.2
        transport.queue_control_response([0]); // revid=0 → FX2 (not FX2LP)

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

    // ── decode_samples (pure function) ───────────────────────────────────────

    #[test]
    fn decode_samples_8bit() {
        let chunk = decode_samples(&[0x01, 0x02, 0x03], false, 10);
        assert_eq!(chunk.logic(), &[0x01, 0x02, 0x03]);
    }

    #[test]
    fn decode_samples_16bit() {
        let chunk = decode_samples(&[0x34, 0x12, 0x78, 0x56, 0xBC, 0x9A], true, 3);
        assert_eq!(chunk.logic().len(), 3);
        assert_eq!(chunk.logic()[0], 0x1234);
        assert_eq!(chunk.logic()[1], 0x5678);
        assert_eq!(chunk.logic()[2], 0x9ABC);
    }

    #[test]
    fn decode_samples_respects_max_samples() {
        let chunk = decode_samples(&[0x01, 0x02, 0x03, 0x04, 0x05, 0x06], false, 2);
        assert_eq!(chunk.logic().len(), 2);
        assert_eq!(chunk.logic()[0], 0x01);
        assert_eq!(chunk.logic()[1], 0x02);
    }

    #[test]
    fn decode_samples_empty_input() {
        let chunk = decode_samples(&[], false, 10);
        assert!(chunk.is_empty());
    }

    // ── start_streaming integration (MockUsbTransport) ──────────────────────────

    /// Helper: create an opened, armed device ready for streaming.
    fn opened_armed_device(transport: MockUsbTransport, channels: u8) -> Fx2lafwDevice {
        let mut dev = Fx2lafwDevice::new(
            DeviceId::new("test"),
            DeviceInfo::new("X", "Y"),
            Box::new(transport),
            Fx2lafwConfig {
                channels,
                sample_rate_hz: 1_000_000.0,
            },
        );
        block_on(dev.open()).unwrap();
        block_on(dev.arm()).unwrap();
        dev
    }

    #[test]
    fn streaming_reads_8bit_from_transport() {
        let mut transport = MockUsbTransport::new();
        transport.queue_control_response([1, 4]); // fw
        transport.queue_control_response([1]); // revid
        transport.queue_control_response([]); // start_acq
        transport.queue_read([0x0F, 0xF0, 0x55]);
        // EOF after first read
        transport.queue_read(&[]);

        let mut dev = opened_armed_device(transport, 8);
        let (tx, mut rx) = mpsc::unbounded();
        let read_loop = block_on(dev.start_streaming(tx)).unwrap();

        let mut pool = futures::executor::LocalPool::new();
        pool.spawner().spawn_local(read_loop).unwrap();

        let chunk = pool.run_until(rx.next()).unwrap();
        assert_eq!(chunk.logic(), &[0x0F, 0xF0, 0x55]);

        drop(rx);
        pool.run_until_stalled();
    }

    #[test]
    fn streaming_reads_16bit_from_transport() {
        let mut transport = MockUsbTransport::new();
        transport.queue_control_response([1, 4]);
        transport.queue_control_response([1]);
        transport.queue_control_response([]);
        transport.queue_read([0x34, 0x12, 0x78, 0x56, 0xBC, 0x9A]);
        transport.queue_read(&[]);

        let mut dev = opened_armed_device(transport, 16);
        assert!(dev.sample_wide);
        let (tx, mut rx) = mpsc::unbounded();
        let read_loop = block_on(dev.start_streaming(tx)).unwrap();

        let mut pool = futures::executor::LocalPool::new();
        pool.spawner().spawn_local(read_loop).unwrap();

        let chunk = pool.run_until(rx.next()).unwrap();
        assert_eq!(chunk.logic().len(), 3);
        assert_eq!(chunk.logic()[0], 0x1234);
        assert_eq!(chunk.logic()[1], 0x5678);
        assert_eq!(chunk.logic()[2], 0x9ABC);

        drop(rx);
        pool.run_until_stalled();
    }

    #[test]
    fn streaming_buffers_partial_16bit_sample() {
        let mut transport = MockUsbTransport::new();
        transport.queue_control_response([1, 4]);
        transport.queue_control_response([1]);
        transport.queue_control_response([]);
        // 6 bytes = 3 complete 16-bit samples (little-endian).
        transport.queue_read([0x01, 0x00, 0x02, 0x00, 0x03, 0x04]);

        let mut dev = opened_armed_device(transport, 16);
        let (tx, mut rx) = mpsc::unbounded();
        let read_loop = block_on(dev.start_streaming(tx)).unwrap();

        let mut pool = futures::executor::LocalPool::new();
        pool.spawner().spawn_local(read_loop).unwrap();

        let chunk = pool.run_until(rx.next()).unwrap();
        assert_eq!(chunk.logic().len(), 3);
        assert_eq!(chunk.logic()[0], 0x0001);
        assert_eq!(chunk.logic()[1], 0x0002);
        assert_eq!(chunk.logic()[2], 0x0403);

        drop(rx);
        pool.run_until_stalled();
    }

    #[test]
    fn streaming_handles_empty_reads_with_counting() {
        // Empty transfers are counted, not treated as immediate EOF.
        // After MAX_EMPTY_TRANSFERS+1 consecutive empties, the loop stops.
        let mut transport = MockUsbTransport::new();
        transport.queue_control_response([1, 4]);
        transport.queue_control_response([1]);
        transport.queue_control_response([]);
        // Queue enough empty reads to trigger the empty-transfer threshold.
        for _ in 0..(MAX_EMPTY_TRANSFERS + 2) {
            transport.queue_read(&[]);
        }

        let mut dev = opened_armed_device(transport, 8);
        let (tx, mut rx) = mpsc::unbounded();
        let read_loop = block_on(dev.start_streaming(tx)).unwrap();

        let mut pool = futures::executor::LocalPool::new();
        pool.spawner().spawn_local(read_loop).unwrap();

        // No data should be produced — all reads are empty.
        let chunk = pool.run_until(rx.next());
        assert!(chunk.is_none() || chunk.unwrap().is_empty());

        pool.run_until_stalled();
    }

    #[test]
    fn streaming_recovers_from_transient_empty_transfer() {
        // A single empty transfer does NOT stop acquisition — empty counting resets
        // when a non-empty transfer arrives.
        let mut transport = MockUsbTransport::new();
        transport.queue_control_response([1, 4]);
        transport.queue_control_response([1]);
        transport.queue_control_response([]);
        transport.queue_read(&[]); // transient empty
        transport.queue_read([0xAA]); // data arrives → counter resets
        transport.queue_read(&[]); // EOF, will be counted

        let mut dev = opened_armed_device(transport, 8);
        let (tx, mut rx) = mpsc::unbounded();
        let read_loop = block_on(dev.start_streaming(tx)).unwrap();

        let mut pool = futures::executor::LocalPool::new();
        pool.spawner().spawn_local(read_loop).unwrap();

        let chunk = pool.run_until(rx.next()).unwrap();
        assert_eq!(chunk.logic(), &[0xAA]);

        drop(rx);
        pool.run_until_stalled();
    }

    #[test]
    fn streaming_stops_via_running_flag() {
        let mut transport = MockUsbTransport::new();
        transport.queue_control_response([1, 4]);
        transport.queue_control_response([1]);
        transport.queue_control_response([]);
        transport.queue_read([0x01, 0x02]);

        let mut dev = opened_armed_device(transport, 8);
        let (tx, mut rx) = mpsc::unbounded();
        let read_loop = block_on(dev.start_streaming(tx)).unwrap();

        let mut pool = futures::executor::LocalPool::new();
        pool.spawner().spawn_local(read_loop).unwrap();

        let chunk = pool.run_until(rx.next()).unwrap();
        assert_eq!(chunk.logic(), &[0x01, 0x02]);

        // Stop via the running flag.
        block_on(dev.stop_streaming()).unwrap();
        assert!(!dev.running.load(Ordering::SeqCst));

        drop(rx);
        pool.run_until_stalled();
    }

    // ── Firmware upload ───────────────────────────────────────────────────────

    #[test]
    fn upload_firmware_sends_control_transfers() {
        let mut transport = MockUsbTransport::new();
        transport.queue_control_response([]); // CPU reset on
        transport.queue_control_response([]); // data write
        transport.queue_control_response([]); // CPU reset off

        // Raw binary data (8 bytes = 1 chunk:
        //   CPU reset on → write data → CPU reset off = 3 transfers)
        let fw = [0x12, 0x34, 0x56, 0x78, 0x9A, 0xBC, 0xDE, 0xF0];
        let result = block_on(upload_firmware(&mut transport, &fw));
        assert!(result.is_ok());

        let ctrl = transport.control_transfers();
        assert_eq!(ctrl.len(), 3, "expected 3 transfers: reset on, data, reset off");

        // 1. CPU reset on
        assert_eq!(ctrl[0].request_type, 0x40);
        assert_eq!(ctrl[0].request, 0xA0);
        assert_eq!(ctrl[0].value, 0xE600);
        assert_eq!(ctrl[0].data, vec![0x01]);

        // 2. Firmware data
        assert_eq!(ctrl[1].request_type, 0x40);
        assert_eq!(ctrl[1].request, 0xA0);
        assert_eq!(ctrl[1].value, 0x0000);
        assert_eq!(ctrl[1].data, fw.to_vec());

        // 3. CPU reset off (starts execution)
        assert_eq!(ctrl[2].request_type, 0x40);
        assert_eq!(ctrl[2].request, 0xA0);
        assert_eq!(ctrl[2].value, 0xE600);
        assert_eq!(ctrl[2].data, vec![0x00]);
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

    // ── Adaptive buffer sizing ─────────────────────────────────────────────

    #[test]
    fn buffer_size_8bit_1mhz() {
        // 1 MHz, 8-bit: 1e6 bytes/s = 1000 bytes/ms. 10ms → 10000, round up to 512 → 10240.
        let sz = compute_buffer_size(1_000_000.0, false);
        assert_eq!(sz, 10240);
    }

    #[test]
    fn buffer_size_8bit_5mhz() {
        // 5 MHz, 8-bit: 5e6 bytes/s = 5000 bytes/ms. 10ms → 50000, round → 50176.
        let sz = compute_buffer_size(5_000_000.0, false);
        assert_eq!(sz, 50176);
    }

    #[test]
    fn buffer_size_16bit_1mhz() {
        // 1 MHz, 16-bit: 2e6 bytes/s = 2000 bytes/ms. 10ms → 20000, round → 20480.
        let sz = compute_buffer_size(1_000_000.0, true);
        assert_eq!(sz, 20480);
    }

    #[test]
    fn buffer_size_rounds_up_to_512() {
        // Very low rate: 100 Hz, 8-bit → 100 bytes/s = 0.1 bytes/ms. 10ms → 1, round to 512.
        let sz = compute_buffer_size(100.0, false);
        assert_eq!(sz, 512);
    }

    #[test]
    fn num_transfers_capped_at_max() {
        // 48 MHz, 16-bit: very high throughput → should hit NUM_SIMUL_TRANSFERS cap.
        let n = compute_num_transfers(48_000_000.0, true);
        assert_eq!(n, NUM_SIMUL_TRANSFERS);
    }

    #[test]
    fn num_transfers_at_least_one() {
        let n = compute_num_transfers(1.0, false);
        assert_eq!(n, 1);
    }

    #[test]
    fn num_transfers_1mhz_8bit() {
        // 1 MHz, 8-bit: buffer=10240, total=500ms*1000B/ms=500KB, 500000/10240≈48 → cap 32.
        let n = compute_num_transfers(1_000_000.0, false);
        assert_eq!(n, 32);
    }
}
