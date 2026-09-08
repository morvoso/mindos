//! Locking the screen when the machine suspends.
//!
//! logind announces a suspend with `PrepareForSleep(true)` and waits for
//! everyone holding a *delay* inhibitor to let go before it goes down. The
//! shell holds one, locks the session the moment the signal arrives and only
//! then releases it. Compositor acknowledgements establish that the session
//! is locked and the displays blanked before the inhibitor is released.
//! Failed or timed-out attempts are logged; logind can enforce its own deadline.
//!
//! The lock itself is the compositor's (`{"type":"lock"}` on the IPC socket),
//! so this runs on its own thread and needs nothing from the main loop.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use futures_util::StreamExt;
use serde_json::{json, Value};
use std::path::Path;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use zbus::zvariant::OwnedFd;

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
pub fn start(lock_on_sleep: Arc<AtomicBool>, events: async_channel::Sender<crate::HostEvent>) {
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
            rt.block_on(run(lock_on_sleep, events));
        })
        .expect("spawn sleep thread");
}

async fn run(lock_on_sleep: Arc<AtomicBool>, events: async_channel::Sender<crate::HostEvent>) {
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

    let socket = crate::ipc::socket_path();
    let mut wake_on_resume = false;
    while let Some(signal) = signals.next().await {
        let going_to_sleep = signal.args().map(|a| a.going_to_sleep).unwrap_or(false);
        if going_to_sleep {
            wake_on_resume = lock_on_sleep.load(Ordering::Relaxed);
            if let Err(e) = prepare_suspend(&socket, wake_on_resume, inhibitor.take(), Duration::from_secs(4)).await {
                tracing::warn!(%e, "could not confirm the screen was locked before suspend");
            }
        } else {
            // Awake again: hold the inhibitor ready for the next time.
            inhibitor = take_inhibitor(&manager).await;
            if wake_on_resume {
                // Reconfirm the lock if preparation failed or logind's delay
                // expired. Authentication still owns unlock.
                if let Err(e) = compositor_actions(&socket, &["lock", "wake"], Duration::from_secs(4)).await {
                    tracing::warn!(%e, "cannot wake the locked displays after suspend");
                }
                wake_on_resume = false;
            }
            let _ = events.send(crate::HostEvent::Resumed).await;
        }
    }
    drop(inhibitor);
}

async fn prepare_suspend(
    socket: &Path, lock: bool, inhibitor: Option<OwnedFd>, deadline: Duration,
) -> Result<(), String> {
    let result = if lock {
        tracing::info!("suspending: waiting for the compositor to lock and blank the displays");
        compositor_actions(socket, &["lock", "blank"], deadline).await
    } else {
        Ok(())
    };
    // A successful write is not an acknowledgement. Keep logind waiting until
    // both replies arrive, or our bounded attempt fails. logind has its own
    // maximum delay and may force sleep sooner if configured below our deadline.
    drop(inhibitor);
    result
}

/// Use a dedicated asynchronous connection: the GTK IPC client's timeout is
/// driven by GLib, which is not running on this sleep watcher's Tokio thread.
async fn compositor_actions(socket: &Path, actions: &[&str], deadline: Duration) -> Result<(), String> {
    tokio::time::timeout(deadline, async {
        let stream = UnixStream::connect(socket).await.map_err(|e| e.to_string())?;
        let mut stream = BufReader::new(stream);
        for (index, action) in actions.iter().enumerate() {
            let id = index + 1;
            let request = format!("{}\n", json!({"id": id, "type": action}));
            stream.get_mut().write_all(request.as_bytes()).await.map_err(|e| e.to_string())?;
            let mut line = Vec::new();
            (&mut stream).take(8193).read_until(b'\n', &mut line).await.map_err(|e| e.to_string())?;
            if line.len() > 8192 || !line.ends_with(b"\n") {
                return Err("incomplete or oversized compositor reply".into());
            }
            let reply: Value = serde_json::from_slice(&line).map_err(|e| e.to_string())?;
            if reply["id"] != id || reply["ok"] != true {
                return Err(format!("compositor rejected {action}: {}", reply["error"]));
            }
            if (*action == "lock" || *action == "blank") && reply["result"]["locked"] != true {
                return Err("compositor did not confirm the session lock".into());
            }
            if *action == "blank" && reply["result"]["stage"] != "blank" {
                return Err("compositor did not confirm blanked displays".into());
            }
        }
        Ok(())
    }).await.map_err(|_| "compositor sleep preparation timed out".to_string())?
}

/// A *delay* inhibitor: logind waits (up to `InhibitDelayMaxSec`, 5 s by
/// default) for the descriptor to close before it suspends.
async fn take_inhibitor(manager: &Login1ManagerProxy<'_>) -> Option<OwnedFd> {
    manager
        .inhibit("sleep", "MindOS", "Locking the screen", "delay")
        .await
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    use std::os::unix::net::UnixStream as StdStream;
    use std::path::PathBuf;
    use std::sync::atomic::AtomicUsize;
    use tokio::net::UnixListener;

    struct Peer {
        path: PathBuf,
        listener: UnixListener,
    }
    impl Peer {
        fn new() -> Self {
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            let path = std::env::temp_dir().join(format!("mindos-sleep-{}-{}.sock", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
            let listener = UnixListener::bind(&path).unwrap();
            Self { path, listener }
        }
    }
    impl Drop for Peer {
        fn drop(&mut self) { let _ = std::fs::remove_file(&self.path); }
    }
    fn inhibitor() -> (Option<OwnedFd>, StdStream) {
        let (held, observer) = StdStream::pair().unwrap();
        observer.set_nonblocking(true).unwrap();
        (Some(std::os::fd::OwnedFd::from(held).into()), observer)
    }
    fn assert_held(observer: &mut StdStream) {
        assert_eq!(observer.read(&mut [0]).unwrap_err().kind(), std::io::ErrorKind::WouldBlock);
    }
    fn assert_released(observer: &mut StdStream) {
        assert_eq!(observer.read(&mut [0]).unwrap(), 0);
    }
    async fn request(stream: &mut BufReader<UnixStream>) -> Value {
        let mut line = String::new();
        stream.read_line(&mut line).await.unwrap();
        serde_json::from_str(&line).unwrap()
    }
    async fn reply(stream: &mut BufReader<UnixStream>, value: Value) {
        stream.get_mut().write_all(format!("{value}\n").as_bytes()).await.unwrap();
    }

    #[tokio::test]
    async fn inhibitor_survives_until_lock_and_blank_are_acknowledged() {
        let peer = Peer::new();
        let (held, mut observer) = inhibitor();
        let path = peer.path.clone();
        let prepare = tokio::spawn(async move { prepare_suspend(&path, true, held, Duration::from_secs(2)).await });
        let (stream, _) = peer.listener.accept().await.unwrap();
        let mut stream = BufReader::new(stream);
        assert_eq!(request(&mut stream).await, json!({"id":1,"type":"lock"}));
        assert_held(&mut observer);
        reply(&mut stream, json!({"id":1,"ok":true,"result":{"locked":true}})).await;
        assert_eq!(request(&mut stream).await, json!({"id":2,"type":"blank"}));
        assert_held(&mut observer);
        reply(&mut stream, json!({"id":2,"ok":true,"result":{"locked":true,"stage":"blank"}})).await;
        prepare.await.unwrap().unwrap();
        assert_released(&mut observer);
    }

    #[tokio::test]
    async fn unresponsive_compositor_has_a_bounded_delay() {
        let peer = Peer::new();
        let (held, mut observer) = inhibitor();
        let path = peer.path.clone();
        let prepare = tokio::spawn(async move { prepare_suspend(&path, true, held, Duration::from_millis(100)).await });
        let (stream, _) = peer.listener.accept().await.unwrap();
        let mut stream = BufReader::new(stream);
        assert_eq!(request(&mut stream).await["type"], "lock");
        assert_held(&mut observer);
        assert!(prepare.await.unwrap().unwrap_err().contains("timed out"));
        assert_released(&mut observer);
    }

    #[tokio::test]
    async fn rejected_or_unconfirmed_lock_does_not_blank() {
        for response in [
            json!({"id":1,"ok":false,"error":"rejected"}),
            json!({"id":1,"ok":true,"result":{"locked":false}}),
            json!({"id":2,"ok":true,"result":{"locked":true}}),
        ] {
            let peer = Peer::new();
            let (held, mut observer) = inhibitor();
            let path = peer.path.clone();
            let prepare = tokio::spawn(async move { prepare_suspend(&path, true, held, Duration::from_secs(2)).await });
            let (stream, _) = peer.listener.accept().await.unwrap();
            let mut stream = BufReader::new(stream);
            request(&mut stream).await;
            reply(&mut stream, response).await;
            assert!(prepare.await.unwrap().is_err());
            assert_released(&mut observer);
            assert_eq!(stream.read(&mut [0]).await.unwrap(), 0);
        }
    }

    #[tokio::test]
    async fn disabled_lock_preference_releases_without_contacting_compositor() {
        let (held, mut observer) = inhibitor();
        prepare_suspend(Path::new("/nonexistent/mindos-sleep.sock"), false, held, Duration::from_secs(2)).await.unwrap();
        assert_released(&mut observer);
    }

    #[tokio::test]
    async fn resume_only_wakes_and_never_unlocks() {
        let peer = Peer::new();
        let path = peer.path.clone();
        let resume = tokio::spawn(async move { compositor_actions(&path, &["lock", "wake"], Duration::from_secs(2)).await });
        let (stream, _) = peer.listener.accept().await.unwrap();
        let mut stream = BufReader::new(stream);
        assert_eq!(request(&mut stream).await, json!({"id":1,"type":"lock"}));
        reply(&mut stream, json!({"id":1,"ok":true,"result":{"locked":true}})).await;
        assert_eq!(request(&mut stream).await, json!({"id":2,"type":"wake"}));
        reply(&mut stream, json!({"id":2,"ok":true,"result":{"locked":true,"stage":"active"}})).await;
        resume.await.unwrap().unwrap();
        assert_eq!(stream.read(&mut [0]).await.unwrap(), 0);
    }
}
