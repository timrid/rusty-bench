//! Firmware assets for fx2lafw devices.
//!
//! Declares every known fx2lafw firmware file via Dioxus' [`asset!()`] macro so
//! the Dioxus CLI (or Trunk) bundles them at build time.  The
//! [`Fx2lafwAssetLoader`] implements [`FirmwareLoader`] using
//! [`read_asset_bytes`], which works on desktop (filesystem) and web (HTTP
//! fetch) with the same API.
//!
//! [`FirmwareLoader`]: rb_drivers::FirmwareLoader
//! [`read_asset_bytes`]: dioxus::asset_resolver::read_asset_bytes

use dioxus::prelude::*;

// ── Firmware assets ───────────────────────────────────────────────────────────

static FW_CYPRESS: Asset = asset!("/firmware-fx2lafw/fx2lafw-cypress-fx2.fw");
static FW_USBEE_AX: Asset = asset!("/firmware-fx2lafw/fx2lafw-cwav-usbeeax.fw");
static FW_USBEE_DX: Asset = asset!("/firmware-fx2lafw/fx2lafw-cwav-usbeedx.fw");
static FW_USBEE_SX: Asset = asset!("/firmware-fx2lafw/fx2lafw-cwav-usbeesx.fw");
static FW_USBEE_ZX: Asset = asset!("/firmware-fx2lafw/fx2lafw-cwav-usbeezx.fw");
static FW_SALEAE: Asset = asset!("/firmware-fx2lafw/fx2lafw-saleae-logic.fw");
static FW_BRAINTECH: Asset = asset!("/firmware-fx2lafw/fx2lafw-braintechnology-usb-lps.fw");
static FW_SIGROK_8CH: Asset = asset!("/firmware-fx2lafw/fx2lafw-sigrok-fx2-8ch.fw");
static FW_SIGROK_16CH: Asset = asset!("/firmware-fx2lafw/fx2lafw-sigrok-fx2-16ch.fw");

/// GPLv2+ license text for the bundled fx2lafw firmware files.
static FW_COPYING: Asset = asset!("/firmware-fx2lafw/COPYING");

// Prevent the linker from discarding statics accessed only via the lookup below.
#[used]
static FIRMWARE_ASSETS: [&Asset; 10] = [
    &FW_CYPRESS,
    &FW_USBEE_AX,
    &FW_USBEE_DX,
    &FW_USBEE_SX,
    &FW_USBEE_ZX,
    &FW_SALEAE,
    &FW_BRAINTECH,
    &FW_SIGROK_8CH,
    &FW_SIGROK_16CH,
    &FW_COPYING,
];

// ── Asset lookup ──────────────────────────────────────────────────────────────

fn lookup_asset(name: &str) -> Option<&'static Asset> {
    match name {
        "fx2lafw-cypress-fx2.fw" => Some(&FW_CYPRESS),
        "fx2lafw-cwav-usbeeax.fw" => Some(&FW_USBEE_AX),
        "fx2lafw-cwav-usbeedx.fw" => Some(&FW_USBEE_DX),
        "fx2lafw-cwav-usbeesx.fw" => Some(&FW_USBEE_SX),
        "fx2lafw-cwav-usbeezx.fw" => Some(&FW_USBEE_ZX),
        "fx2lafw-saleae-logic.fw" => Some(&FW_SALEAE),
        "fx2lafw-braintechnology-usb-lps.fw" => Some(&FW_BRAINTECH),
        "fx2lafw-sigrok-fx2-8ch.fw" => Some(&FW_SIGROK_8CH),
        "fx2lafw-sigrok-fx2-16ch.fw" => Some(&FW_SIGROK_16CH),
        "COPYING" => Some(&FW_COPYING),
        _ => None,
    }
}

// ── Loader ────────────────────────────────────────────────────────────────────

/// Resolves fx2lafw firmware via Dioxus' asset system.
pub struct Fx2lafwAssetLoader;

impl Fx2lafwAssetLoader {
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl Default for Fx2lafwAssetLoader {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait(?Send)]
impl rb_drivers::FirmwareLoader for Fx2lafwAssetLoader {
    async fn load_firmware(&self, name: &str) -> Result<Vec<u8>, String> {
        let asset = lookup_asset(name)
            .ok_or_else(|| format!("unknown firmware file: {name}"))?;

        dioxus::asset_resolver::read_asset_bytes(asset)
            .await
            .map_err(|e| format!("failed to load firmware '{name}': {e}"))
    }
}
