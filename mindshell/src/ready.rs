// Copyright (c) 2026 Black Arrow Software, LLC. All rights reserved.
// Licensed under the MindOS Software License; see LICENSE at the repository root.

//! Telling the service manager when the desktop is on screen.
//!
//! `mindos-shell.service` is `Type=notify` and `session-startup` waits for it
//! before it releases the session's autostart applications. That handshake is
//! the whole reason this exists: Discord, Steam and the tray applets wait for
//! nothing of their own accord, and until the shell reported ready they came
//! up *in front of* the compositor's startup screen -- the user watched
//! Discord paint over the operating system still loading behind it.
//!
//! Ready means the desktop view mapped and the compositor took its startup
//! screen down, not that the process got as far as `main`. If that never
//! happens the shell says ready anyway after [`FALLBACK`]: a shell that cannot
//! put a desktop up is something to read the journal about, not a reason to
//! hold every application in the session for the unit's whole start timeout
//! and then have systemd kill it into a restart loop.

use std::os::linux::net::SocketAddrExt;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::net::{SocketAddr, UnixDatagram};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use gtk4::glib;

/// How long the session waits for a desktop before starting its applications
/// without one. Long enough for a cold boot on a slow disk -- the desktop is
/// up in well under a second once the compositor is not starving it -- and
/// short enough that a broken shell does not read as a hung login.
pub const FALLBACK: Duration = Duration::from_secs(15);

static TOLD: AtomicBool = AtomicBool::new(false);

/// Report the desktop up, once. The second output's desktop view, and the
/// fallback after a view already reported, both find the work done.
pub fn desktop_up() {
    if TOLD.swap(true, Ordering::SeqCst) {
        return;
    }
    match notify("READY=1\n") {
        Ok(true) => tracing::debug!("told the service manager the desktop is up"),
        Ok(false) => {}
        Err(e) => tracing::warn!(%e, "cannot tell the service manager the desktop is up"),
    }
}

/// Say ready anyway if no desktop has mapped by [`FALLBACK`], so a shell that
/// cannot draw one still holds the session up only as long as it is worth
/// waiting. Call once, after GTK is initialised, from the main thread.
pub fn arm_fallback() {
    glib::timeout_add_local_once(FALLBACK, || {
        if TOLD.load(Ordering::SeqCst) {
            return;
        }
        tracing::warn!(
            seconds = FALLBACK.as_secs(),
            "no desktop after {}s; releasing the session's applications anyway",
            FALLBACK.as_secs()
        );
        desktop_up();
    });
}

/// Send one `sd_notify` datagram. `Ok(false)` means nobody is listening --
/// mindshell run from a terminal, which is the usual case while developing.
fn notify(state: &str) -> std::io::Result<bool> {
    let Some(path) = std::env::var_os("NOTIFY_SOCKET") else {
        return Ok(false);
    };
    let socket = UnixDatagram::unbound()?;
    let bytes = path.as_bytes();
    // systemd spells an abstract socket with a leading '@' where the address
    // has its leading NUL.
    if let Some(name) = bytes.strip_prefix(b"@") {
        socket.send_to_addr(state.as_bytes(), &SocketAddr::from_abstract_name(name)?)?;
    } else {
        socket.send_to(state.as_bytes(), path)?;
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The whole session hangs on this one datagram: without it systemd holds
    /// the shell in `activating` until `TimeoutStartSec`, kills it, and every
    /// autostart application has waited that long for nothing.
    #[test]
    fn the_desktop_reports_ready_once() {
        let dir = std::env::temp_dir().join(format!("mindshell-ready-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("notify");
        let manager = UnixDatagram::bind(&path).unwrap();
        manager.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        std::env::set_var("NOTIFY_SOCKET", &path);

        desktop_up();
        let mut buf = [0u8; 64];
        let n = manager.recv(&mut buf).unwrap();
        assert_eq!(&buf[..n], b"READY=1\n", "systemd reads exactly this");

        // The second monitor's desktop, and the fallback behind it, both find
        // the session already released.
        desktop_up();
        manager.set_nonblocking(true).unwrap();
        assert!(manager.recv(&mut buf).is_err(), "ready is reported once");

        // Run from a terminal there is no service manager to tell, and that
        // is not a failure. (One test, because the two share the variable.)
        std::env::remove_var("NOTIFY_SOCKET");
        assert!(matches!(notify("READY=1\n"), Ok(false)), "nobody to tell is not an error");

        std::fs::remove_dir_all(&dir).ok();
    }
}
