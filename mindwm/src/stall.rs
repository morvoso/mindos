// Copyright (c) 2026 Black Arrow Software, LLC. All rights reserved.
// Licensed under the MindOS Software License; see LICENSE at the repository root.

//! Which part of the compositor the event loop is in.
//!
//! Everything the compositor does -- drawing every display, moving the
//! pointer, answering clients -- happens on one thread, between returns to
//! the event loop. A call that blocks there freezes all of it at once, and
//! the thread that would say so is the one that is stuck. The watcher in
//! `watchdog` notices from outside; this is what lets it say which part of
//! the compositor made the call, and name the DRM request it is waiting in.

use std::sync::atomic::{AtomicU8, Ordering};

static PHASE: AtomicU8 = AtomicU8::new(Phase::Loop as u8);

/// The part of the compositor the event loop is in. Only the work that can
/// block for long is marked; everything else reads as `Loop`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Phase {
    Loop = 0,
    Refresh,
    Clients,
    Input,
    Render,
    Vblank,
    Recovery,
    Hotplug,
    Session,
    XWayland,
    Tray,
    Ipc,
}

impl Phase {
    const ALL: [Phase; 12] = [
        Phase::Loop,
        Phase::Refresh,
        Phase::Clients,
        Phase::Input,
        Phase::Render,
        Phase::Vblank,
        Phase::Recovery,
        Phase::Hotplug,
        Phase::Session,
        Phase::XWayland,
        Phase::Tray,
        Phase::Ipc,
    ];

    /// Reads after "the compositor is stuck".
    fn describe(self) -> &'static str {
        match self {
            Phase::Loop => "handling an event",
            Phase::Refresh => "updating windows and focus",
            Phase::Clients => "handling Wayland client requests",
            Phase::Input => "handling input",
            Phase::Render => "drawing a frame",
            Phase::Vblank => "finishing a page flip",
            Phase::Recovery => "resetting a display",
            Phase::Hotplug => "looking at connected displays",
            Phase::Session => "switching the session in or out",
            Phase::XWayland => "talking to XWayland",
            Phase::Tray => "reading tray icons from XWayland",
            Phase::Ipc => "answering the shell",
        }
    }
}

/// Marks the part of the compositor running until the guard is dropped.
pub struct Guard(u8);

impl Drop for Guard {
    fn drop(&mut self) {
        PHASE.store(self.0, Ordering::Relaxed);
    }
}

pub fn enter(phase: Phase) -> Guard {
    Guard(PHASE.swap(phase as u8, Ordering::Relaxed))
}

/// What the event loop is doing, as it reads after "the compositor is
/// stuck". Read from the watcher thread.
pub fn current() -> &'static str {
    Phase::ALL
        .get(PHASE.load(Ordering::Relaxed) as usize)
        .copied()
        .unwrap_or(Phase::Loop)
        .describe()
}

/// DRM's requests by name; any other ioctl by its type and number.
pub fn ioctl_name(request: u64) -> String {
    let kind = ((request >> 8) & 0xff) as u8;
    let nr = (request & 0xff) as u8;
    if kind == b'd' {
        let name = match nr {
            0x09 => "DRM_IOCTL_GEM_CLOSE",
            0x2d => "DRM_IOCTL_PRIME_HANDLE_TO_FD",
            0x2e => "DRM_IOCTL_PRIME_FD_TO_HANDLE",
            0xa1 => "DRM_IOCTL_MODE_GETCRTC",
            0xa2 => "DRM_IOCTL_MODE_SETCRTC",
            0xa6 => "DRM_IOCTL_MODE_GETENCODER",
            0xa7 => "DRM_IOCTL_MODE_GETCONNECTOR",
            0xaa => "DRM_IOCTL_MODE_GETPROPERTY",
            0xac => "DRM_IOCTL_MODE_GETPROPBLOB",
            0xaf => "DRM_IOCTL_MODE_RMFB",
            0xb0 => "DRM_IOCTL_MODE_PAGE_FLIP",
            0xb2 => "DRM_IOCTL_MODE_CREATE_DUMB",
            0xb8 => "DRM_IOCTL_MODE_ADDFB2",
            0xb9 => "DRM_IOCTL_MODE_OBJ_GETPROPERTIES",
            0xbc => "DRM_IOCTL_MODE_ATOMIC",
            0xbd => "DRM_IOCTL_MODE_CREATEPROPBLOB",
            0xbe => "DRM_IOCTL_MODE_DESTROYPROPBLOB",
            0xc3 => "DRM_IOCTL_SYNCOBJ_WAIT",
            0xca => "DRM_IOCTL_SYNCOBJ_TIMELINE_WAIT",
            0xcb => "DRM_IOCTL_SYNCOBJ_QUERY",
            0xd0 => "DRM_IOCTL_MODE_CLOSEFB",
            nr if (0x40..0xa0).contains(&nr) => return format!("driver-specific DRM ioctl 0x{nr:02x}"),
            _ => return format!("DRM ioctl 0x{nr:02x}"),
        };
        return name.into();
    }
    if kind.is_ascii_graphic() {
        format!("ioctl '{}' 0x{nr:02x}", kind as char)
    } else {
        format!("ioctl 0x{request:x}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ioctls_are_named() {
        assert_eq!(ioctl_name(0xc03864bc), "DRM_IOCTL_MODE_ATOMIC");
        assert_eq!(ioctl_name(0xc05064a7), "DRM_IOCTL_MODE_GETCONNECTOR");
        assert_eq!(ioctl_name(0xc0106442), "driver-specific DRM ioctl 0x42");
        assert_eq!(ioctl_name(0xc0204600 | 0x2a), "ioctl 'F' 0x2a");
    }

    #[test]
    fn phases_round_trip() {
        for (i, phase) in Phase::ALL.iter().enumerate() {
            assert_eq!(*phase as usize, i);
        }
    }

    #[test]
    fn the_phase_is_restored_when_its_guard_goes() {
        let before = current();
        {
            let _render = enter(Phase::Render);
            assert_eq!(current(), "drawing a frame");
            {
                let _vblank = enter(Phase::Vblank);
                assert_eq!(current(), "finishing a page flip");
            }
            assert_eq!(current(), "drawing a frame");
        }
        assert_eq!(current(), before);
    }
}
