//! Application settings with persistent storage.
//!
//! Uses [`dioxus_sdk_storage::LocalStorage`] to store settings across
//! application restarts (localStorage on WASM, file on desktop via `set_dir!`).

use dioxus::prelude::*;
use dioxus_sdk_storage::{LocalStorage, use_storage};
use crate::app_state::Theme;

// ── Settings struct ──────────────────────────────────────────────────────────

/// All user-configurable application settings.
///
/// Add new fields here; they will automatically be persisted.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct AppSettings {
    /// Color theme: System, Light, or Dark.
    pub theme: Theme,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            theme: Theme::System,
        }
    }
}

// ── Hook ─────────────────────────────────────────────────────────────────────

/// Returns a persistent signal for application settings.
///
/// The signal is backed by [`LocalStorage`] and survives app restarts.
/// Use `.write()` to update settings — changes are automatically saved.
pub fn use_settings() -> Signal<AppSettings> {
    use_storage::<LocalStorage, AppSettings>("rb-settings".to_string(), AppSettings::default)
}
