// Copyright (c) 2026 Black Arrow Software, LLC. All rights reserved.
// Licensed under the MindOS Software License; see LICENSE at the repository root.

//! Noticing when the event loop stops turning.
//!
//! Everything the compositor does -- drawing every display, moving the
//! pointer, answering clients -- happens on one thread, between returns to
//! the event loop. A call that blocks there freezes all of it at once, and
//! the log says nothing, because the thread that would say something is the
//! one that is stuck. So a second thread watches. The loop counts its turns;
//! when a turn has run for a second without the thread going back to wait
//! for events, the watcher writes down what the thread is doing -- the system
//! call, the file it is on, where in the kernel it sleeps, and which part of
//! the compositor made the call -- and writes again once the loop moves.
//!
//! Waiting in `epoll_wait` is the loop being idle, never a stall.

use std::sync::atomic::{AtomicI32, AtomicU64, AtomicU8, Ordering};
use std::time::{Duration, Instant};

use tracing::{info, warn};

/// How often the watcher looks at the event loop.
const SAMPLE: Duration = Duration::from_millis(100);
/// A turn this long is a stall worth writing down.
const STALL: Duration = Duration::from_secs(1);
/// When a stall is written down again while it lasts.
const REPORT_AT: [Duration; 4] = [
    STALL,
    Duration::from_secs(5),
    Duration::from_secs(15),
    Duration::from_secs(60),
];

static TURNS: AtomicU64 = AtomicU64::new(0);
static PHASE: AtomicU8 = AtomicU8::new(Phase::Loop as u8);
static WATCHING: AtomicI32 = AtomicI32::new(0);

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

/// One turn of the event loop is over.
pub fn turn() {
    TURNS.fetch_add(1, Ordering::Relaxed);
}

/// Start watching the calling thread, which must be the one that runs the
/// event loop. Does nothing when called a second time.
pub fn start() {
    // SAFETY: gettid has no preconditions and cannot fail.
    let tid = unsafe { libc::gettid() };
    if WATCHING.swap(tid, Ordering::SeqCst) != 0 {
        return;
    }
    if let Err(err) = std::thread::Builder::new()
        .name("mindwm-stall".into())
        .spawn(move || watch(tid))
    {
        warn!(%err, "no stall watcher: a frozen event loop will not be reported");
    }
}

fn watch(tid: i32) {
    let syscall_path = format!("/proc/self/task/{tid}/syscall");
    let wchan_path = format!("/proc/self/task/{tid}/wchan");
    let mut last_turn = TURNS.load(Ordering::Relaxed);
    // When the turn under way was first seen running, and how many times
    // it has been written down.
    let mut busy_since: Option<Instant> = None;
    let mut reports = 0usize;
    loop {
        std::thread::sleep(SAMPLE);
        let now = Instant::now();
        let turn = TURNS.load(Ordering::Relaxed);
        let Ok(syscall) = std::fs::read_to_string(&syscall_path) else {
            // The thread is gone: the compositor is on its way out.
            return;
        };
        let idle = is_idle(&syscall);
        let moved = turn != last_turn;
        last_turn = turn;
        if moved || idle {
            if let Some(since) = busy_since.take().filter(|_| reports > 0) {
                info!(stalled_for = ?now.duration_since(since), "the event loop is running again");
            }
            reports = 0;
            // A turn that ended since the last look may already have been
            // followed by the next one: it began somewhere in between.
            busy_since = (!idle).then_some(now);
            continue;
        }
        let since = *busy_since.get_or_insert(now);
        let stalled = now.duration_since(since);
        if stalled < report_due(reports) {
            continue;
        }
        reports += 1;
        let phase = Phase::ALL
            .get(PHASE.load(Ordering::Relaxed) as usize)
            .copied()
            .unwrap_or(Phase::Loop);
        let wchan = std::fs::read_to_string(&wchan_path).unwrap_or_default();
        warn!(
            stalled_for = ?stalled,
            "the compositor is stuck {}: {}; every display and the pointer are frozen until it returns",
            phase.describe(),
            describe_call(&syscall, wchan.trim(), fd_target),
        );
    }
}

/// How long a stall lasts before it is written down for the `reports`+1-th
/// time: at 1, 5, 15 and 60 seconds, then every minute.
fn report_due(reports: usize) -> Duration {
    let last = REPORT_AT.len() - 1;
    REPORT_AT
        .get(reports)
        .copied()
        .unwrap_or(REPORT_AT[last] + Duration::from_secs(60) * (reports - last) as u32)
}

/// Whether the thread is waiting for events, which is what an idle loop does.
fn is_idle(syscall: &str) -> bool {
    matches!(
        syscall.split_whitespace().next().and_then(|nr| nr.parse::<i64>().ok()),
        Some(nr) if IDLE_SYSCALLS.contains(&nr)
    )
}

#[cfg(target_arch = "x86_64")]
const IDLE_SYSCALLS: &[i64] = &[232, 281, 441]; // epoll_wait, epoll_pwait, epoll_pwait2
#[cfg(not(target_arch = "x86_64"))]
const IDLE_SYSCALLS: &[i64] = &[22, 441]; // epoll_pwait, epoll_pwait2

fn fd_target(fd: u64) -> Option<String> {
    std::fs::read_link(format!("/proc/self/fd/{fd}"))
        .ok()
        .map(|path| path.display().to_string())
}

/// What `/proc/<tid>/syscall` and `wchan` say, in words: "in ioctl
/// DRM_IOCTL_MODE_ATOMIC on /dev/dri/card1, sleeping in nv_drm_atomic_commit".
fn describe_call(syscall: &str, wchan: &str, target: impl Fn(u64) -> Option<String>) -> String {
    let mut fields = syscall.split_whitespace();
    let first = fields.next().unwrap_or("");
    if first == "running" {
        return "running compositor code (not waiting on the kernel)".into();
    }
    let sleeping = if wchan.is_empty() || wchan == "0" {
        String::new()
    } else {
        format!(", sleeping in {wchan}")
    };
    let Ok(nr) = first.parse::<i64>() else {
        return format!("in an unknown state ({})", syscall.trim());
    };
    if nr < 0 {
        return format!("blocked outside a system call{sleeping}");
    }
    let args: Vec<u64> = fields
        .take(6)
        .map(|arg| u64::from_str_radix(arg.trim_start_matches("0x"), 16).unwrap_or(0))
        .collect();
    let arg = |i: usize| args.get(i).copied().unwrap_or(0);
    let name = syscall_name(nr).map(str::to_string).unwrap_or_else(|| format!("system call {nr}"));
    let mut text = format!("in {name}");
    if nr == SYS_IOCTL {
        text.push(' ');
        text.push_str(&ioctl_name(arg(1)));
    }
    if FD_SYSCALLS.contains(&nr) {
        match target(arg(0)) {
            Some(path) => text.push_str(&format!(" on {path}")),
            None => text.push_str(&format!(" on fd {}", arg(0))),
        }
    }
    text + &sleeping
}

#[cfg(target_arch = "x86_64")]
const SYS_IOCTL: i64 = 16;
#[cfg(not(target_arch = "x86_64"))]
const SYS_IOCTL: i64 = 29;

/// System calls whose first argument is a file descriptor.
#[cfg(target_arch = "x86_64")]
const FD_SYSCALLS: &[i64] = &[0, 1, 3, 16, 17, 18, 19, 20, 44, 45, 46, 47, 72, 74, 75];
#[cfg(not(target_arch = "x86_64"))]
const FD_SYSCALLS: &[i64] = &[29, 57, 63, 64, 65, 66, 67, 68, 82, 83, 206, 207, 211, 212];

#[cfg(target_arch = "x86_64")]
fn syscall_name(nr: i64) -> Option<&'static str> {
    Some(match nr {
        0 => "read",
        1 => "write",
        3 => "close",
        7 => "poll",
        9 => "mmap",
        11 => "munmap",
        16 => "ioctl",
        17 => "pread64",
        18 => "pwrite64",
        19 => "readv",
        20 => "writev",
        23 => "select",
        35 => "nanosleep",
        44 => "sendto",
        45 => "recvfrom",
        46 => "sendmsg",
        47 => "recvmsg",
        61 => "wait4",
        72 => "fcntl",
        74 => "fsync",
        75 => "fdatasync",
        202 => "futex",
        230 => "clock_nanosleep",
        257 => "openat",
        270 => "pselect6",
        271 => "ppoll",
        _ => return None,
    })
}

#[cfg(not(target_arch = "x86_64"))]
fn syscall_name(nr: i64) -> Option<&'static str> {
    Some(match nr {
        29 => "ioctl",
        57 => "close",
        63 => "read",
        64 => "write",
        73 => "ppoll",
        98 => "futex",
        101 => "nanosleep",
        115 => "clock_nanosleep",
        206 => "sendto",
        207 => "recvfrom",
        211 => "sendmsg",
        212 => "recvmsg",
        _ => return None,
    })
}

/// DRM's requests by name; any other ioctl by its type and number.
fn ioctl_name(request: u64) -> String {
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

    #[cfg(target_arch = "x86_64")]
    #[test]
    fn waiting_for_events_is_idle() {
        assert!(is_idle("232 0x5 0x7ffd 0x20 0xffffffff 0x0 0x0 0x7ffd0 0x7f00"));
        assert!(is_idle("281 0x5 0x7ffd 0x20 0xffffffff 0x0 0x8 0x7ffd0 0x7f00"));
        assert!(!is_idle("running"));
        assert!(!is_idle("16 0xc 0xc03864bc 0x7ffd 0x0 0x0 0x0 0x7ffd0 0x7f00"));
        assert!(!is_idle("-1 0x7ffd0 0x7f00"));
    }

    #[cfg(target_arch = "x86_64")]
    #[test]
    fn an_atomic_commit_is_named_with_its_device() {
        let text = describe_call(
            "16 0xc 0xc03864bc 0x7ffc1 0x0 0x0 0x0 0x7ffd0 0x7f00",
            "nv_drm_atomic_commit",
            |fd| (fd == 12).then(|| "/dev/dri/card1".to_string()),
        );
        assert_eq!(
            text,
            "in ioctl DRM_IOCTL_MODE_ATOMIC on /dev/dri/card1, sleeping in nv_drm_atomic_commit"
        );
    }

    #[cfg(target_arch = "x86_64")]
    #[test]
    fn other_calls_read_plainly() {
        assert_eq!(
            describe_call("1 0x1 0x5555 0x40 0x0 0x0 0x0 0x7ffd0 0x7f00", "unix_stream_sendmsg", |_| {
                Some("socket:[4242]".into())
            }),
            "in write on socket:[4242], sleeping in unix_stream_sendmsg"
        );
        assert_eq!(
            describe_call("202 0x7f 0x80 0x0 0x0 0x0 0x0 0x7ffd0 0x7f00", "0", |_| None),
            "in futex"
        );
        assert_eq!(describe_call("running", "0", |_| None), "running compositor code (not waiting on the kernel)");
        assert_eq!(describe_call("-1 0x7ffd0 0x7f00", "", |_| None), "blocked outside a system call");
    }

    #[test]
    fn ioctls_are_named() {
        assert_eq!(ioctl_name(0xc03864bc), "DRM_IOCTL_MODE_ATOMIC");
        assert_eq!(ioctl_name(0xc05064a7), "DRM_IOCTL_MODE_GETCONNECTOR");
        assert_eq!(ioctl_name(0xc0106442), "driver-specific DRM ioctl 0x42");
        assert_eq!(ioctl_name(0xc0204600 | 0x2a), "ioctl 'F' 0x2a");
    }

    #[test]
    fn long_stalls_are_reported_less_often() {
        let due: Vec<u64> = (0..7).map(|n| report_due(n).as_secs()).collect();
        assert_eq!(due, [1, 5, 15, 60, 120, 180, 240]);
    }

    #[test]
    fn phases_round_trip() {
        for (i, phase) in Phase::ALL.iter().enumerate() {
            assert_eq!(*phase as usize, i);
        }
    }
}
