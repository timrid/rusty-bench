//! Logic Analyzer acquisition orchestration.
//!
//! Free functions that operate on [`AppState`](crate::app_state::AppState):
//! start (async drain for UI / sync for tests), stop, clear, and queries.

use std::cell::RefCell;
use std::rc::Rc;

use dioxus::prelude::{spawn as dioxus_spawn, Signal};
use futures::channel::{mpsc, oneshot};
use futures::StreamExt;
#[cfg(test)]
use futures::task::LocalSpawnExt;
use rb_core::{run_acquisition, AcquisitionCommand, AcquisitionState, DeviceHandle};
use rb_device::DeviceId;
use rb_model::SampleChunk;

use crate::app_state::AppState;
use crate::tab_state::TabId;

use super::acquisition::{AcquisitionConfig, DeviceAcquisition};

pub type AppStateRef = Rc<RefCell<AppState>>;

// ── Test executor ────────────────────────────────────────────────────────────

#[cfg(test)]
thread_local! {
    static TEST_POOL: RefCell<(futures::executor::LocalPool, futures::executor::LocalSpawner)> = RefCell::new({
        let pool = futures::executor::LocalPool::new();
        let spawner = pool.spawner();
        (pool, spawner)
    });
}

// ── Platform spawn ───────────────────────────────────────────────────────────

/// Spawns a future on the platform's executor (UI-only path).
/// Uses Dioxus's spawn which works on desktop (tokio LocalSet) and WASM.
fn spawn_future(fut: impl std::future::Future<Output = ()> + 'static) {
    dioxus_spawn(fut);
}

// ═══════════════════════════════════════════════════════════════════════════════
//  Async drain tasks (UI — event-driven, no polling)
// ═══════════════════════════════════════════════════════════════════════════════

/// Spawns a background task that drains `data_rx` into the tab's traces
/// and bumps `data_version` on each chunk, triggering re-renders.
pub fn spawn_drain_task(
    mut data_rx: mpsc::UnboundedReceiver<SampleChunk>,
    app_ref: AppStateRef,
    tab_id: TabId,
    mut data_version: Signal<u64>,
) {
    spawn_future(async move {
        while let Some(chunk) = data_rx.next().await {
            let mut app = app_ref.borrow_mut();
            if let Some(tab) = app.tabs.get_mut(&tab_id) {
                if let Some(acq) = tab.logic_analyzer_mut().acquisition.as_mut() {
                    acq.push_chunk(&chunk);
                }
            }
            drop(app);
            data_version += 1;
        }
    });
}

/// Spawns a task that awaits a handle-return oneshot (no polling).
fn spawn_return_task(
    return_rx: oneshot::Receiver<(DeviceId, DeviceHandle)>,
    app_ref: AppStateRef,
) {
    spawn_future(async move {
        if let Ok((device_id, handle)) = return_rx.await {
            app_ref.borrow_mut().device_manager.return_handle(device_id, handle);
        }
    });
}

// ═══════════════════════════════════════════════════════════════════════════════
//  Acquisition spawn
// ═══════════════════════════════════════════════════════════════════════════════

/// Spawns an acquisition future and returns a [`DeviceAcquisition`].
///
/// If `drain_info` is `Some`, sets up async drain + handle-return tasks
/// (no polling needed). If `None`, the caller must drain manually (tests).
fn spawn(
    _app: &mut AppState,
    config: &AcquisitionConfig,
    mut handle: DeviceHandle,
    device_id: DeviceId,
    drain_info: Option<(AppStateRef, TabId, Signal<u64>)>,
) -> DeviceAcquisition {
    config.apply_to_handle(&mut handle);
    let (analog, digital) = config.build_traces();

    // ── Channel setup (same for all platforms) ────────────────────────────
    let (cmd_tx, cmd_rx) = mpsc::unbounded::<AcquisitionCommand>();
    let (data_tx, data_rx) = mpsc::unbounded::<SampleChunk>();
    let (return_tx, return_rx) = oneshot::channel();

    let _ = cmd_tx.unbounded_send(AcquisitionCommand::SetSampleRate(config.sample_rate_hz));
    let _ = cmd_tx.unbounded_send(AcquisitionCommand::Start);

    // ── Spawn acquisition future ──────────────────────────────────────────
    let fut = run_acquisition(handle, cmd_rx, Some(data_tx));
    if drain_info.is_some() {
        // UI: Dioxus spawn works on desktop (tokio) and WASM (wasm-bindgen).
        dioxus_spawn(async move {
            let h = fut.await;
            let _ = return_tx.send((device_id, h));
        });
    } else {
        #[cfg(test)]
        TEST_POOL.with(|p| {
            let _ = p.borrow().1.spawn_local(async move {
                let h = fut.await;
                let _ = return_tx.send((device_id, h));
            });
        });
    }

    // ── Build DeviceAcquisition ───────────────────────────────────────────
    if let Some((app_ref, tab_id, ver)) = drain_info {
        // UI path: async drain + async handle return
        spawn_drain_task(data_rx, app_ref.clone(), tab_id, ver);
        spawn_return_task(return_rx, app_ref);

        DeviceAcquisition {
            analog,
            digital,
            state: AcquisitionState::Running,
            sample_count: 0,
            cmd_tx,
            data_rx: None,
            config: config.clone(),
        }
    } else {
        // Test path: keep data_rx for manual drain
        _app.device_manager.register_pending_return(return_rx);

        DeviceAcquisition {
            analog,
            digital,
            state: AcquisitionState::Running,
            sample_count: 0,
            cmd_tx,
            data_rx: Some(data_rx),
            config: config.clone(),
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
//  Public control API
// ═══════════════════════════════════════════════════════════════════════════════

/// Start acquisition for the given tab (UI version with async drain).
///
/// Spawns background tasks to drain data and collect the handle return
/// — no polling needed.
pub fn start(app_ref: &AppStateRef, tab_id: TabId, data_version: Signal<u64>) {
    let mut app = app_ref.borrow_mut();
    _start_impl(
        &mut app,
        tab_id,
        Some((app_ref.clone(), tab_id, data_version)),
    );
}

/// Start acquisition (test version — sync, manual drain via [`drain_all`]).
#[cfg(test)]
pub fn start_sync(app: &mut AppState, tab_id: TabId) {
    _start_impl(app, tab_id, None);
}

fn _start_impl(
    app: &mut AppState,
    tab_id: TabId,
    drain_info: Option<(AppStateRef, TabId, Signal<u64>)>,
) {
    // Check if the assigned device is still connected
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

    // If already acquiring, just reset and restart
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

    // Fresh start: spawn acquisition future
    let device_id = app
        .tabs
        .get(&tab_id)
        .and_then(|t| t.assigned_device_id().cloned());
    let handle = device_id
        .as_ref()
        .and_then(|did| app.device_manager.take_handle(did));

    if let (Some(device_id), Some(handle)) = (device_id, handle) {
        let config = app
            .tabs
            .get(&tab_id)
            .map(|t| t.logic_analyzer().acquisition_config.clone())
            .unwrap_or_default();

        let acq = spawn(app, &config, handle, device_id, drain_info);
        if let Some(tab) = app.tabs.get_mut(&tab_id) {
            tab.logic_analyzer_mut().acquisition = Some(acq);
        }
    }
}

/// Stop acquisition for the given tab.
pub fn stop(app: &mut AppState, tab_id: TabId) {
    if let Some(tab) = app.tabs.get_mut(&tab_id) {
        if let Some(acq) = tab.logic_analyzer_mut().acquisition.as_mut() {
            acq.send_command(AcquisitionCommand::Stop);
            acq.state = AcquisitionState::Stopped;
        }
    }
}

/// Clear the acquisition from a tab (stop + drop).
pub fn clear_acquisition(app: &mut AppState, tab_id: TabId) {
    if let Some(tab) = app.tabs.get_mut(&tab_id) {
        if let Some(acq) = tab.logic_analyzer_mut().acquisition.as_mut() {
            acq.send_command(AcquisitionCommand::Stop);
            acq.state = AcquisitionState::Stopped;
        }
        tab.logic_analyzer_mut().acquisition = None;
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
//  Queries
// ═══════════════════════════════════════════════════════════════════════════════

/// Drains acquisition data for the active tab (sync, for tests).
pub fn drain_all(app: &mut AppState) -> bool {
    let Some(tab) = app.tabs.get_mut(&app.active_tab) else {
        return false;
    };
    let Some(acq) = tab.logic_analyzer_mut().acquisition.as_mut() else {
        return false;
    };
    let before = acq.sample_count();
    acq.drain();
    acq.sample_count() > before
}

/// Returns the acquisition for a tab.
pub fn acq_for_tab(app: &AppState, tab_id: TabId) -> Option<&DeviceAcquisition> {
    app.tabs
        .get(&tab_id)
        .and_then(|t| t.logic_analyzer().acquisition.as_ref())
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

/// Drives the test executor.
#[cfg(test)]
pub fn pump_executor() {
    TEST_POOL.with(|pool| pool.borrow_mut().0.run_until_stalled());
}
