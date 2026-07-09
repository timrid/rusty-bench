//! Logic Analyzer acquisition orchestration.
//!
//! Free functions that operate on [`AppState`](crate::app_state::AppState):
//! start (async drain for UI), stop, clear, and queries.

use std::cell::RefCell;
use std::rc::Rc;

use dioxus::prelude::{spawn as dioxus_spawn, Signal};
use futures::channel::{mpsc, oneshot};
use futures::StreamExt;
use rb_core::{run_acquisition, DeviceHandle};
use rb_device::DeviceId;
use rb_model::SampleChunk;

use crate::app_state::AppState;
use crate::tab_state::TabId;

use super::AcquisitionState;

pub type AppStateRef = Rc<RefCell<AppState>>;

// ── Platform spawn ───────────────────────────────────────────────────────────

fn spawn_future(fut: impl std::future::Future<Output = ()> + 'static) {
    dioxus_spawn(fut);
}

// ═══════════════════════════════════════════════════════════════════════════════
//  Async drain task
// ═══════════════════════════════════════════════════════════════════════════════

/// Spawns a background task that drains `gui_rx` into the tab's traces
/// and bumps `data_version` on each chunk, triggering re-renders.
fn spawn_drain_task(
    mut gui_rx: mpsc::UnboundedReceiver<SampleChunk>,
    app_ref: AppStateRef,
    tab_id: TabId,
    mut data_version: Signal<u64>,
) {
    spawn_future(async move {
        while let Some(chunk) = gui_rx.next().await {
            let mut app = app_ref.borrow_mut();
            if let Some(tab) = app.tabs.get_mut(&tab_id) {
                tab.logic_analyzer_mut().push_chunk(&chunk);
            }
            drop(app);
            data_version += 1;
        }
    });
}

// ═══════════════════════════════════════════════════════════════════════════════
//  Public control API
// ═══════════════════════════════════════════════════════════════════════════════

/// Start acquisition for the given tab.
pub fn start(app_ref: &AppStateRef, tab_id: TabId, data_version: Signal<u64>) {
    let app_ref_owned = app_ref.clone();
    let mut app = app_ref_owned.borrow_mut();

    // Check if the assigned device is still connected.
    let disconnected = app
        .tabs
        .get(&tab_id)
        .is_some_and(|t| {
            t.assigned_device_id()
                .is_some_and(|did| !app.device_manager.is_connected(did))
        });
    if disconnected {
        return;
    }

    // If already acquiring, drop the old data_tx to stop the current stream.
    if let Some(tab) = app.tabs.get_mut(&tab_id) {
        let la = tab.logic_analyzer_mut();
        la.data_tx = None;
        la.reset_traces();
        la.acq_state = AcquisitionState::Running;
    }

    // Get device handle and config.
    let device_id = app
        .tabs
        .get(&tab_id)
        .and_then(|t| t.assigned_device_id().cloned());
    let handle = device_id
        .as_ref()
        .and_then(|did| app.device_manager.take_handle(did));
    let config = app
        .tabs
        .get(&tab_id)
        .map(|t| t.logic_analyzer().acquisition_config.clone())
        .unwrap_or_default();

    let (device_id, handle) = match (device_id, handle) {
        (Some(did), Some(h)) => (did, h),
        _ => return,
    };

    // Build fresh traces in the tab.
    let (analog, digital) = config.build_traces();
    {
        let tab = app.tabs.get_mut(&tab_id).unwrap();
        let la = tab.logic_analyzer_mut();
        la.analog = analog;
        la.digital = digital;
        la.sample_count = 0;
        la.acq_state = AcquisitionState::Running;
    }

    // Channels for data flow.
    let (data_tx_device, data_rx_device) = mpsc::unbounded::<SampleChunk>();
    let (gui_tx, gui_rx) = mpsc::unbounded::<SampleChunk>();
    let (return_tx, return_rx) = oneshot::channel();

    // Keep a clone of data_tx_device in the tab for stop signal.
    {
        let tab = app.tabs.get_mut(&tab_id).unwrap();
        tab.logic_analyzer_mut().data_tx = Some(data_tx_device.clone());
    }

    let sample_rate = config.sample_rate_hz;

    // Spawn drain task (receives from gui_tx via gui_rx).
    spawn_drain_task(gui_rx, app_ref_owned.clone(), tab_id, data_version);

    // Spawn task that awaits handle return.
    let return_app_ref = app_ref_owned.clone();
    spawn_future(async move {
        if let Ok((did, h)) = return_rx.await {
            return_app_ref.borrow_mut().device_manager.return_handle(did, h);
        }
    });

    // Spawn the acquisition orchestration.
    let update_app_ref = app_ref_owned.clone();
    dioxus_spawn(async move {
        let mut handle = handle;
        // Set sample rate and arm.
        if let Some(la) = handle.device_mut().as_logic_analyzer_mut() {
            let _ = la.set_sample_rate_hz(sample_rate).await;
            let _ = la.arm().await;
        }
        if let Some(scope) = handle.device_mut().as_oscilloscope_mut() {
            let _ = scope.set_sample_rate_hz(sample_rate).await;
            let _ = scope.arm().await;
        }

        // Start streaming: device pushes to data_tx_device.
        if let Some(src) = handle.device_mut().as_acquisition_source_mut() {
            if let Ok(read_loop) = src.start_streaming(data_tx_device).await {
                // run_acquisition polls read_loop and forwards data_rx_device → gui_tx.
                run_acquisition(read_loop, data_rx_device, Some(gui_tx)).await;
            }
        }

        // After run_acquisition exits: clean stop.
        if let Some(src) = handle.device_mut().as_acquisition_source_mut() {
            let _ = src.stop_streaming().await;
        }
        if let Some(la) = handle.device_mut().as_logic_analyzer_mut() {
            let _ = la.stop().await;
        }
        if let Some(scope) = handle.device_mut().as_oscilloscope_mut() {
            let _ = scope.stop().await;
        }

        // Update tab state to Stopped (if not already in error).
        {
            let mut app = update_app_ref.borrow_mut();
            if let Some(tab) = app.tabs.get_mut(&tab_id) {
                let la = tab.logic_analyzer_mut();
                if la.acq_state == AcquisitionState::Running {
                    la.acq_state = AcquisitionState::Stopped;
                }
                la.data_tx = None;
            }
        }

        let _ = return_tx.send((device_id, handle));
    });
}

/// Stop acquisition for the given tab.
pub fn stop(app: &mut AppState, tab_id: TabId) {
    if let Some(tab) = app.tabs.get_mut(&tab_id) {
        let la = tab.logic_analyzer_mut();
        la.data_tx = None;
        if la.acq_state == AcquisitionState::Running {
            la.acq_state = AcquisitionState::Stopped;
        }
    }
}

/// Clear the acquisition from a tab (stop + drop traces).
pub fn clear_acquisition(app: &mut AppState, tab_id: TabId) {
    if let Some(tab) = app.tabs.get_mut(&tab_id) {
        let la = tab.logic_analyzer_mut();
        la.data_tx = None;
        la.acq_state = AcquisitionState::Idle;
        la.analog.clear();
        la.digital = None;
        la.sample_count = 0;
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
//  Queries
// ═══════════════════════════════════════════════════════════════════════════════

/// Returns the acquisition state for a device, searching across all tabs.
pub fn device_acq_state(app: &AppState, id: &DeviceId) -> Option<AcquisitionState> {
    for tab in app.tabs.values() {
        if tab.assigned_device_id() == Some(id) {
            return Some(tab.logic_analyzer().acq_state.clone());
        }
    }
    None
}

/// Returns the acquisition state of the active tab.
pub fn active_tab_acq_state(app: &AppState) -> AcquisitionState {
    let Some(tab) = app.tabs.get(&app.active_tab) else {
        return AcquisitionState::Idle;
    };
    tab.logic_analyzer().acq_state.clone()
}
