//! Platform-specific title bar theme control.
//!
//! On Windows, calls `DwmSetWindowAttribute` to set immersive dark mode.
//! On other platforms, this is a no-op.

/// Set the title bar to dark or light appearance.
///
/// # Platform support
/// - **Windows 10 1809+**: Sets immersive dark mode via DWM.
/// - **macOS / Linux**: No-op (system controls the title bar).
#[cfg(target_os = "windows")]
pub fn set_title_bar_theme(dark: bool) {
    // SAFETY: The HWND comes from the dioxus window, which is guaranteed
    // to be valid. `DwmSetWindowAttribute` is a well-known Win32 API.
    #[allow(unsafe_code)]
    unsafe {
        use raw_window_handle::{HasWindowHandle, RawWindowHandle};

        let win = dioxus_desktop::window();
        // win.window is a wry::Window wrapped in an Rc. Borrow it.
        let win_ref = &win.window;

        match win_ref.window_handle() {
            Ok(handle) => {
                let hwnd = match handle.as_raw() {
                    RawWindowHandle::Win32(w) => w.hwnd.get() as isize,
                    _ => {
                        log::warn!("title_bar: unsupported window handle type");
                        return;
                    }
                };
                set_dark_mode_for_hwnd(hwnd, dark);
            }
            Err(e) => {
                log::warn!("title_bar: failed to get window handle: {e:?}");
            }
        }
    }
}

#[cfg(not(target_os = "windows"))]
pub fn set_title_bar_theme(_dark: bool) {
    // No-op on non-Windows platforms.
}

// ── Windows implementation ───────────────────────────────────────────────────

#[cfg(target_os = "windows")]
#[allow(unsafe_code)]
unsafe fn set_dark_mode_for_hwnd(hwnd: isize, dark: bool) {
    use std::ffi::c_void;

    // DWMWA_USE_IMMERSIVE_DARK_MODE = 20 (Windows 10 20H1+)
    // Before 20H1, value 19 was used.
    const DWMWA_USE_IMMERSIVE_DARK_MODE: i32 = 20;

    unsafe extern "system" {
        fn DwmSetWindowAttribute(
            hwnd: isize,
            dw_attribute: u32,
            pv_attribute: *const c_void,
            cb_attribute: u32,
        ) -> i32;
    }

    let enabled: i32 = if dark { 1 } else { 0 };
    let result = unsafe {
        DwmSetWindowAttribute(
            hwnd,
            DWMWA_USE_IMMERSIVE_DARK_MODE as u32,
            &enabled as *const i32 as *const c_void,
            std::mem::size_of::<i32>() as u32,
        )
    };

    if result != 0 {
        // Try the older attribute (19) for pre-20H1 Windows 10.
        const DWMWA_USE_IMMERSIVE_DARK_MODE_OLD: i32 = 19;
        let result2 = unsafe {
            DwmSetWindowAttribute(
                hwnd,
                DWMWA_USE_IMMERSIVE_DARK_MODE_OLD as u32,
                &enabled as *const i32 as *const c_void,
                std::mem::size_of::<i32>() as u32,
            )
        };
        if result2 != 0 {
            log::debug!("title_bar: DwmSetWindowAttribute failed (hr=0x{result2:08X})");
        }
    }
}
