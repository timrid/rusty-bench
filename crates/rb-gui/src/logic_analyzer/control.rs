//! Logic Analyzer acquisition orchestration.
//!
//! Free functions that operate on [`AppState`](crate::app_state::AppState)
//! and encapsulate all Logic-Analyzer-specific acquisition lifecycle:
//! start, stop, drain, spawn, and query.

use std::cell::RefCell;

use futures::channel::mpsc;
use rb_core::{
    run_acquisition, AcquisitionCommand, AcquisitionState, DeviceHandle,
};
use rb_device::DeviceId;

use crate::app_state::AppState;
use crate::tab_state::TabId;

#[cfg(not(any(feature = "native", target_arch = "wasm32")))]
use {futures::executor::LocalPool, futures::task::LocalSpawnExt};

use super::acquisition::{AcquisitionConfig, DeviceAcquisition};

// ── Test executor ────────────────────────────────────────────────────────────

#[cfg(not(any(feature = "native", target_arch = "wasm32")))]
thread_local! {
    static TEST_POOL: RefCell<(LocalPool, futures::executor::LocalSpawner)> = RefCell::new({
        let pool = LocalPool::new();
        let spawner = pool.spawner();
        (pool, spawner)
    });
}

// ── Acquisition control ──────────────────────────────────────────────────────

/// Start acquisition for the given tab.
/// Takes the device handle from [`DeviceManager`] and spawns the acquisition future.
pub fn start(app: &mut AppState, tab_id: TabId) {
    // Check if we need to connect first.
    let need_connect = {
        let tab = app.tabs.get(&tab_id);
        tab.is_some_and(|t| {
            t.assigned_device_id()
                .is_some_and(|did| !app.device_manager.is_connected(did))
        })
    };

    if need_connect {
        return;
    }

    // Check if already acquiring — re-run.
    let already_acquiring = app
        .tabs
        .get(&tab_id)
        .is_some_and(|t| t.logic_analyzer().acquisition.as_ref().is_some());

    if already_acquiring {
        if let Some(tab) = app.tabs.get_mut(&tab_id) {
            if let Some(acq) = tab.logic_analyzer_mut().acquisition.as_mut() {
                let rate = acq.config.sample_rate_hz;
                acq.send_command(AcquisitionCommand::SetSampleRate(rate));
                acq.reset_traces();
                acq.send_command(AcquisitionCommand::Start);
                acq.state = AcquisitionState::Running;
            }
        }
        return;
    }

    // Take the handle from DeviceManager.
    let device_id = app
        .tabs
        .get(&tab_id)
        .and_then(|t| t.assigned_device_id().cloned());

    let handle = device_id
        .as_ref()
        .and_then(|did| app.device_manager.take_handle(did));

    if let Some(handle) = handle {
        let device_id = device_id.unwrap();
        let acq = spawn(app, handle, device_id);
        if let Some(tab) = app.tabs.get_mut(&tab_id) {
            tab.logic_analyzer_mut().acquisition = Some(acq);
        }
    }
}

/// Stop acquisition for the given tab.
pub fn stop(app: &mut AppState, tab_id: TabId) {
    let Some(tab) = app.tabs.get_mut(&tab_id) else { return };
    if let Some(acq) = tab.logic_analyzer_mut().acquisition.as_mut() {
        acq.send_command(AcquisitionCommand::Stop);
        acq.state = AcquisitionState::Stopped;
    }
}

/// Clear the acquisition from a tab (stop first, then drop the handle).
pub fn clear_acquisition(app: &mut AppState, tab_id: TabId) {
    if let Some(tab) = app.tabs.get_mut(&tab_id) {
        if let Some(acq) = tab.logic_analyzer_mut().acquisition.as_mut() {
            acq.send_command(AcquisitionCommand::Stop);
            acq.state = AcquisitionState::Stopped;
        }
        tab.logic_analyzer_mut().acquisition = None;
    }
}

// ── Deferred actions (called from AppState::apply_pending_actions) ───────────

/// Non-blocking version of [`start`] — same logic, used for deferred starts.
pub(crate) fn apply_start(app: &mut AppState, tab_id: TabId) {
    let need_connect = app
        .tabs
        .get(&tab_id)
        .is_some_and(|t| {
            t.assigned_device_id()
                .is_some_and(|did| !app.device_manager.is_connected(did))
        });

    if need_connect {
        return;
    }

    let already_acquiring = app
        .tabs
        .get(&tab_id)
        .is_some_and(|t| t.logic_analyzer().acquisition.as_ref().is_some());

    if already_acquiring {
        if let Some(tab) = app.tabs.get_mut(&tab_id) {
            if let Some(acq) = tab.logic_analyzer_mut().acquisition.as_mut() {
                let rate = acq.config.sample_rate_hz;
                acq.send_command(AcquisitionCommand::SetSampleRate(rate));
                acq.reset_traces();
                acq.send_command(AcquisitionCommand::Start);
                acq.state = AcquisitionState::Running;
            }
        }
        return;
    }

    let device_id = app
        .tabs
        .get(&tab_id)
        .and_then(|t| t.assigned_device_id().cloned());

    let handle = device_id
        .as_ref()
        .and_then(|did| app.device_manager.take_handle(did));

    if let Some(handle) = handle {
        let device_id = device_id.unwrap();
        let acq = spawn(app, handle, device_id);
        if let Some(tab) = app.tabs.get_mut(&tab_id) {
            tab.logic_analyzer_mut().acquisition = Some(acq);
        }
    }
}

/// Non-blocking version of [`stop`].
pub(crate) fn apply_stop(app: &mut AppState, tab_id: TabId) {
    let Some(tab) = app.tabs.get_mut(&tab_id) else { return };
    if let Some(acq) = tab.logic_analyzer_mut().acquisition.as_mut() {
        acq.send_command(AcquisitionCommand::Stop);
        acq.state = AcquisitionState::Stopped;
    }
}

// ── Acquisition spawning ─────────────────────────────────────────────────────

/// Spawns an acquisition future and returns the [`DeviceAcquisition`] handle.
/// The device handle is returned to [`DeviceManager`] when the future completes.
#[allow(unused_mut)]
pub fn spawn(
    app: &mut AppState,
    mut handle: DeviceHandle,
    device_id: DeviceId,
) -> DeviceAcquisition {
    // Read config from the tab that owns this device.
    let config = app
        .tabs
        .values()
        .find(|t| t.assigned_device_id() == Some(&device_id))
        .map(|t| t.logic_analyzer().acquisition_config.clone())
        .unwrap_or_default();

    // Rebuild handle traces to match config.
    config.apply_to_handle(&mut handle);

    let (analog, digital) = config.build_traces();

    #[cfg(target_arch = "wasm32")]
    {
        let web_handle = rb_core::runtime::web::spawn_local(handle);
        let _ = web_handle
            .commands
            .unbounded_send(AcquisitionCommand::SetSampleRate(config.sample_rate_hz));
        let _ = web_handle.commands.unbounded_send(AcquisitionCommand::Start);
        return DeviceAcquisition {
            analog,
            digital,
            state: AcquisitionState::Running,
            sample_count: 0,
            cmd_tx: web_handle.commands,
            data_rx: web_handle.data,
            config,
        };
    }

    #[cfg(not(target_arch = "wasm32"))]
    {
        let (cmd_tx, cmd_rx) = mpsc::unbounded::<AcquisitionCommand>();
        let (data_tx, data_rx) = mpsc::unbounded();

        let (return_tx, return_rx) = futures::channel::oneshot::channel();
        app.device_manager.register_pending_return(return_rx);

        let fut = run_acquisition(handle, cmd_rx, Some(data_tx));
        #[cfg(feature = "native")]
        {
            tokio::spawn(async move {
                let handle = fut.await;
                let _ = return_tx.send((device_id, handle));
            });
        }
        #[cfg(not(feature = "native"))]
        {
            TEST_POOL.with(|pool| {
                pool.borrow().1.spawn_local(async move {
                    let handle = fut.await;
                    let _ = return_tx.send((device_id, handle));
                }).expect("spawn");
            });
        }

        let _ = cmd_tx.unbounded_send(AcquisitionCommand::SetSampleRate(config.sample_rate_hz));
        let _ = cmd_tx.unbounded_send(AcquisitionCommand::Start);

        DeviceAcquisition {
            analog,
            digital,
            state: AcquisitionState::Running,
            sample_count: 0,
            cmd_tx,
            data_rx,
            config,
        }
    }
}

// ── Queries ──────────────────────────────────────────────────────────────────

/// Returns the acquisition for a tab.
pub fn acq_for_tab(app: &AppState, tab_id: TabId) -> Option<&DeviceAcquisition> {
    app.tabs.get(&tab_id).and_then(|t| t.logic_analyzer().acquisition.as_ref())
}

/// Returns a mutable reference to the acquisition for a tab.
pub fn acq_for_tab_mut(app: &mut AppState, tab_id: TabId) -> Option<&mut DeviceAcquisition> {
    app.tabs.get_mut(&tab_id).and_then(|t| t.logic_analyzer_mut().acquisition.as_mut())
}

/// Returns the acquisition state for a device, searching across all tabs.
pub fn device_acq_state(app: &AppState, id: &DeviceId) -> Option<AcquisitionState> {
    for tab in app.tabs.values() {
        if tab.assigned_device_id() == Some(id) {
            if let Some(acq) = tab.logic_analyzer().acquisition.as_ref() {
                return Some(acq.state().clone());
            }
        }
    }
    None
}

/// Returns the acquisition state of the active tab.
pub fn active_tab_acq_state(app: &AppState) -> AcquisitionState {
    let Some(tab) = app.tabs.get(&app.active_tab) else {
        return AcquisitionState::Idle;
    };
    if let Some(acq) = tab.logic_analyzer().acquisition.as_ref() {
        acq.state().clone()
    } else {
        tab.assigned_device_id()
            .and_then(|did| app.device_manager.device_handle(did))
            .map(|h| h.state().clone())
            .unwrap_or(AcquisitionState::Idle)
    }
}

/// Drains acquisition data for the active tab only.
/// Returns true if any new data arrived.
pub fn drain_all(app: &mut AppState) -> bool {
    let Some(tab) = app.tabs.get_mut(&app.active_tab) else { return false };
    let Some(acq) = tab.logic_analyzer_mut().acquisition.as_mut() else { return false };
    let before = acq.sample_count();
    acq.drain();
    acq.sample_count() > before
}

/// Drives the test executor (no-op on desktop and WASM).
pub fn pump_executor() {
    #[cfg(not(any(feature = "native", target_arch = "wasm32")))]
    TEST_POOL.with(|pool| pool.borrow_mut().0.run_until_stalled());
}
