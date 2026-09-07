//! Locking the screen when the machine suspends.
//!
//! logind announces a suspend with `PrepareForSleep(true)` and waits for
//! everyone holding a *delay* inhibitor to let go before it goes down. The
//! shell holds one, locks the session the moment the signal arrives and only
//! then releases it, so the machine never suspends with the desktop still on
//! screen — which is what somebody would see for a moment when it wakes.
//!
//! The lock itself is the compositor's (`{"type":"lock"}` on the IPC socket),
//! so this runs on its own thread and needs nothing from the main loop.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use futures_util::StreamExt;
use serde_json::json;
use zbus::zvariant::OwnedFd;

use crate::ipc::IpcClient;

#[zbus::proxy(
    interface = "org.freedesktop.login1.Manager",
    default_service = "org.freedesktop.login1",
    default_path = "/org/freedesktop/login1"
)]
trait Login1Manager {
    /// `mode` is `block` or `delay`; the returned descriptor holds the
    /// inhibitor for as long as it stays open.
    fn inhibit(&self, what: &str, who: &str, why: &str, mode: &str) -> zbus::Result<OwnedFd>;

    /// `true` just before the machine suspends, `false` once it is back.
    #[zbus(signal)]
    fn prepare_for_sleep(&self, going_to_sleep: bool) -> zbus::Result<()>;
}

/// Start the watcher. `lock_on_sleep` is read afresh on every suspend, so the
/// Settings toggle takes effect without a restart.
pub fn start(ipc: IpcClient, lock_on_sleep: Arc<AtomicBool>) {
    std::thread::Builder::new()
        .name("mindshell-sleep".into())
        .spawn(move || {
            let rt = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
                Ok(rt) => rt,
                Err(e) => {
                    tracing::warn!(%e, "cannot start the sleep watcher");
                    return;
                }
            };
            rt.block_on(run(ipc, lock_on_sleep));
        })
        .expect("spawn sleep thread");
}

async fn run(ipc: IpcClient, lock_on_sleep: Arc<AtomicBool>) {
    let bus = match zbus::Connection::system().await {
        Ok(bus) => bus,
        Err(e) => {
            tracing::warn!(%e, "no system bus: the screen will not lock on suspend");
            return;
        }
    };
    let manager = match Login1ManagerProxy::new(&bus).await {
        Ok(manager) => manager,
        Err(e) => {
            tracing::warn!(%e, "logind is unavailable: the screen will not lock on suspend");
            return;
        }
    };
    let mut signals = match manager.receive_prepare_for_sleep().await {
        Ok(signals) => signals,
        Err(e) => {
            tracing::warn!(%e, "cannot watch logind for suspend");
            return;
        }
    };

    let mut inhibitor = take_inhibitor(&manager).await;
    if inhibitor.is_none() {
        tracing::warn!("logind refused the sleep inhibitor; the lock may come up only after the resume");
    }

    while let Some(signal) = signals.next().await {
        let going_to_sleep = signal.args().map(|a| a.going_to_sleep).unwrap_or(false);
        if going_to_sleep {
            if lock_on_sleep.load(Ordering::Relaxed) {
                tracing::info!("suspending: locking the screen");
                if let Err(e) = ipc.send(json!({ "type": "lock" })) {
                    tracing::warn!(%e, "cannot lock the screen before the suspend");
                }
            }
            // Let go so the machine can actually go down.
            inhibitor = None;
        } else {
            // Awake again: hold the inhibitor ready for the next time.
            inhibitor = take_inhibitor(&manager).await;
        }
    }
    drop(inhibitor);
}

/// A *delay* inhibitor: logind waits (up to `InhibitDelayMaxSec`, 5 s by
/// default) for the descriptor to close before it suspends.
async fn take_inhibitor(manager: &Login1ManagerProxy<'_>) -> Option<OwnedFd> {
    manager
        .inhibit("sleep", "MindOS", "Locking the screen", "delay")
        .await
        .ok()
}
