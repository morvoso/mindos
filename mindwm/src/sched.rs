// Copyright (c) 2026 Black Arrow Software, LLC. All rights reserved.
// Licensed under the MindOS Software License; see LICENSE at the repository root.

//! What the compositor asks the scheduler for.
//!
//! Everything the user sees goes through one thread: the event loop reads
//! input, decides what the frame looks like, draws it and hands it to the
//! display controller. It needs very little CPU — a repaint is a fraction of
//! a millisecond — but it needs it *now*, sixty to two hundred and forty
//! times a second. A compositor that is merely one runnable thread among
//! many gets its turn when the scheduler comes round, and the frame it was
//! going to draw is late.
//!
//! That is not a hypothetical: `libinput` reports "event processing lagging
//! behind by N ms" whenever the loop is slow to drain the input fd, and this
//! machine's journal has those from 22 ms to 1794 ms, the long ones while a
//! compiler had every core busy. GameMode renices the *game* to -10 (see
//! `gamemode.ini`), so during the one workload MindOS exists for, the
//! compositor was the lower-priority of the two.
//!
//! So the loop thread runs at a negative nice value, one step above what
//! GameMode gives the game: the display server outranks what it displays.
//! Realtime scheduling would be the other answer and is what KWin asks rtkit
//! for, but this kernel builds without `CONFIG_RT_GROUP_SCHED` and therefore
//! without RT bandwidth control (`sched_rt_runtime_us` equals the period), so
//! a SCHED_RR loop that ever spun would take the machine with it and leave no
//! way in. A nice value cannot do that.
//!
//! Nothing here is required for correctness: every call is best-effort and
//! says what it got.

use std::process::Command;

use tracing::{debug, info, warn};

/// What the loop thread asks for, best first. `-11` is one better than
/// GameMode's `renice=10`; the rest are what a machine whose `limits.conf`
/// has not been updated (or a foreign one) will still grant.
const WANTED: [i32; 4] = [-11, -10, -5, -1];

/// The nice value a program the user starts should run at. The compositor's
/// own priority is not an inheritance.
const NORMAL: i32 = 0;

/// Set the calling thread's nice value. `PRIO_PROCESS` with `who = 0` is the
/// calling thread on Linux, not the whole process, which is what makes it
/// possible to prioritise the loop without prioritising the worker threads.
fn set_nice(value: i32) -> Result<(), std::io::Error> {
    // SAFETY: a plain syscall on the calling thread with no memory involved.
    let rc = unsafe { libc::setpriority(libc::PRIO_PROCESS, 0, value) };
    if rc == -1 {
        // -1 is also a legal nice value, so errno is the only answer.
        let err = std::io::Error::last_os_error();
        if err.raw_os_error() != Some(0) {
            return Err(err);
        }
    }
    Ok(())
}

/// Ask for the event loop's priority. Call from the loop thread once the
/// backend is up: every thread created afterwards inherits this value, and
/// the GPU driver's own threads are made during renderer initialisation.
pub fn prioritise_loop() {
    for wanted in WANTED {
        match set_nice(wanted) {
            Ok(()) => {
                info!(nice = wanted, "the event loop runs above the programs it draws");
                return;
            }
            Err(err) if err.raw_os_error() == Some(libc::EACCES) || err.raw_os_error() == Some(libc::EPERM) => {
                debug!(nice = wanted, "not allowed to ask for this priority");
            }
            Err(err) => {
                warn!(nice = wanted, %err, "cannot set the event loop's priority");
                return;
            }
        }
    }
    // RLIMIT_NICE is 20 - (the lowest nice allowed); MindOS grants it to the
    // desktop group in limits.conf. Without it the loop competes with every
    // build job and every game on equal terms, and it will lose some frames.
    warn!("the event loop runs at the default priority: a busy machine will make it late");
}

/// Give the calling thread the ordinary desktop priority. Worker threads
/// created after [`prioritise_loop`] inherit the loop's value and should not
/// keep it; none of them are on the frame path.
pub fn normal_priority() {
    if let Err(err) = set_nice(NORMAL) {
        debug!(%err, "cannot return this thread to the normal priority");
    }
}

/// Run a child process at the desktop's priority rather than the
/// compositor's.
///
/// A program started from the Mind bar, a terminal, the shell or a key
/// binding is forked from the loop thread and would otherwise inherit its
/// nice value — every application on the system would run at the
/// compositor's priority, which is both wrong and pointless. The reset
/// happens in the child between fork and exec; raising one's own nice value
/// is always permitted, so it cannot fail for lack of privilege.
pub fn at_desktop_priority(command: &mut Command) -> &mut Command {
    use std::os::unix::process::CommandExt;
    // SAFETY: `setpriority` is a syscall, so it is async-signal-safe and
    // allocates nothing; that is the whole contract of `pre_exec`.
    unsafe {
        command.pre_exec(|| {
            libc::setpriority(libc::PRIO_PROCESS, 0, NORMAL);
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_loop_asks_for_more_than_gamemode_gives_the_game() {
        // gamemode.ini has renice=10, which is nice -10 for the game.
        assert_eq!(WANTED[0], -11);
        assert!(WANTED.windows(2).all(|w| w[0] < w[1]), "best first");
        assert!(WANTED.iter().all(|&n| n < NORMAL));
    }

    #[test]
    fn a_thread_can_always_give_its_priority_back() {
        // Lowering one's own priority needs no privilege, so this holds
        // wherever the tests run.
        normal_priority();
        let nice = unsafe {
            *libc::__errno_location() = 0;
            libc::getpriority(libc::PRIO_PROCESS, 0)
        };
        assert_eq!(nice, NORMAL);
    }
}
