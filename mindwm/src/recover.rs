//! Surviving a panic on the event loop.
//!
//! Everything the compositor does happens in a callback on one loop, and a
//! panic in any of them unwinds straight out of `main`. The process exits,
//! `mindos-session` sees it go and takes the session down with it: every
//! window, every game, every unsaved thing the user had open, gone because
//! one `unwrap` somewhere met a `None`.
//!
//! Those `None`s are not hypothetical. A client can ask to be resized after
//! the button is already up; a monitor can be unplugged between a request
//! being sent and being read; Xwayland can die mid-frame and take its
//! connection with it. Each of those is a normal thing for a desktop to live
//! through, and each of them used to be a way to end the session. The
//! handlers this module protects have been gone through one by one, but they
//! are not the only code that runs here: smithay's own handlers, the
//! renderer, and the client protocol machinery all run on this thread too,
//! and none of them were written on the understanding that a panic ends the
//! user's session.
//!
//! So the loop catches them. A callback that panics loses whatever it was
//! doing -- one frame, one request, one window's idea of where it is -- and
//! the next turn of the loop carries on. That is not free: the state it was
//! half-way through changing stays half-changed. It is still the better of
//! the two outcomes by a wide margin, because the alternative is not a
//! consistent compositor, it is no compositor.
//!
//! What it must not do is spin. A source that panics every time it is polled
//! would panic forever, at whatever rate the loop can manage, filling the
//! journal and pinning a core. So the panics are counted: a run of them
//! close together is not a hiccup, it is a loop that cannot get past
//! something, and then the compositor stops on purpose and says why.

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

use tracing::error;

/// Two panics further apart than this are two separate accidents. Closer
/// than this and the loop is going round on the same one.
const SPIN_WINDOW: Duration = Duration::from_secs(1);

/// How many panics in a row, each inside `SPIN_WINDOW` of the last, before
/// the compositor gives up. Generous, because a burst of frames or client
/// requests that all trip over the same thing is exactly what a recoverable
/// panic looks like, and each one costs a fraction of a second at worst.
const SPIN_LIMIT: u32 = 16;

/// Install a panic handler that says where the panic was in the journal,
/// with a backtrace, before the unwinding starts.
///
/// The default handler writes to stderr, which the session does capture, but
/// only prints a backtrace when `RUST_BACKTRACE` was set in the environment
/// the compositor was started with -- and nobody sets that before the crash
/// they did not know was coming. A compositor panic is rare enough that
/// walking the stack costs nothing anyone will notice, and without it the
/// line in the journal names a file and says nothing about how it was
/// reached.
pub fn say_where_panics_happen() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let at = info
            .location()
            .map(|at| format!("{}:{}", at.file(), at.line()))
            .unwrap_or_else(|| "somewhere unknown".to_string());
        error!(
            at = %at,
            backtrace = %std::backtrace::Backtrace::force_capture(),
            "the compositor panicked: {}",
            message(info)
        );
        previous(info);
    }));
}

/// What the panic said, out of the payload every panic carries.
fn message(info: &std::panic::PanicHookInfo<'_>) -> String {
    let payload = info.payload();
    if let Some(s) = payload.downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "no message".to_string()
    }
}

/// Taking a lock without minding that a panic once happened while it was held.
pub trait LockAnyway<T> {
    /// The guard, poisoned or not.
    ///
    /// A `Mutex` is marked poisoned forever once a thread panics while
    /// holding it, and `lock().unwrap()` turns that into a panic of its own.
    /// Before the loop caught panics, that hardly mattered: the first panic
    /// had already ended the session. Now it matters a great deal -- one
    /// survived panic would poison a lock the compositor takes every frame,
    /// every frame after it would panic on the lock rather than the original
    /// fault, and the spin guard would end the session anyway, over the
    /// wrong thing and with the real cause buried sixteen panics back.
    ///
    /// What these locks hold is the compositor's own bookkeeping -- a
    /// client's commit deadlines, the pointer's hotspot, which capture
    /// buffers are in use. A half-updated one of those costs a frame. It is
    /// the same trade the module makes everywhere else, for the same reason.
    fn lock_anyway(&self) -> MutexGuard<'_, T>;
}

impl<T> LockAnyway<T> for Mutex<T> {
    fn lock_anyway(&self) -> MutexGuard<'_, T> {
        self.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// The loop's record of how badly things have been going.
#[derive(Debug, Default)]
pub struct Recovery {
    /// Panics in a row, each close behind the last.
    run: u32,
    /// When the last one was.
    last: Option<Instant>,
}

impl Recovery {
    /// Run one turn of the event loop, surviving a panic inside it.
    ///
    /// Returns what the turn returned, or `None` if it panicked. The panic
    /// has already been reported by the hook above by the time this returns.
    pub fn turn<T>(&mut self, turn: impl FnOnce() -> T) -> Option<T> {
        // The state this borrows is left however the panic left it. That is
        // the trade the module comment describes, and it is deliberate.
        match catch_unwind(AssertUnwindSafe(turn)) {
            Ok(value) => {
                self.run = 0;
                Some(value)
            }
            Err(_) => {
                self.note(Instant::now());
                None
            }
        }
    }

    /// Record a panic at `now`, taken as an argument so the counting can be
    /// tested without waiting a second per case.
    fn note(&mut self, now: Instant) {
        let close_behind = self
            .last
            .is_some_and(|last| now.saturating_duration_since(last) < SPIN_WINDOW);
        self.run = if close_behind { self.run + 1 } else { 1 };
        self.last = Some(now);
        error!(
            in_a_row = self.run,
            "the compositor carried on after a panic; the frame or request it was in was lost"
        );
    }

    /// Whether the loop is going round on the same panic and should stop.
    ///
    /// Stopping ends the session, which is the thing this whole module
    /// exists to avoid -- but a compositor that panics sixteen times in a
    /// row is not serving anyone either, and stopping at least leaves a
    /// journal that says so instead of one that fills the disk.
    pub fn giving_up(&self) -> bool {
        self.run >= SPIN_LIMIT
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The whole point: a callback that panics does not end the process, and
    /// the next one still runs.
    #[test]
    fn a_panic_costs_a_turn_and_not_the_session() {
        let mut recovery = Recovery::default();
        assert_eq!(recovery.turn(|| 1 + 1), Some(2));
        assert_eq!(recovery.turn(|| -> i32 { panic!("a client asked for the impossible") }), None);
        assert_eq!(recovery.turn(|| 1 + 1), Some(2), "the loop keeps going");
        assert!(!recovery.giving_up());
    }

    /// A run of panics far apart is a run of separate accidents, and the
    /// compositor lives through all of them.
    #[test]
    fn panics_that_are_not_a_spin_never_give_up() {
        let mut recovery = Recovery::default();
        let mut when = Instant::now();
        for _ in 0..SPIN_LIMIT * 4 {
            when += SPIN_WINDOW * 2;
            recovery.note(when);
            assert!(!recovery.giving_up(), "one panic a minute is not a spin");
        }
    }

    /// A source that panics every time it is polled is a loop that cannot
    /// make progress, and the compositor stops rather than spin on it.
    #[test]
    fn a_loop_that_only_panics_stops() {
        let mut recovery = Recovery::default();
        let mut when = Instant::now();
        for _ in 1..SPIN_LIMIT {
            when += SPIN_WINDOW / 4;
            recovery.note(when);
            assert!(!recovery.giving_up(), "still worth trying");
        }
        when += SPIN_WINDOW / 4;
        recovery.note(when);
        assert!(recovery.giving_up(), "sixteen in a row is not a hiccup");
    }

    /// One good turn is enough to say the spin is over.
    #[test]
    fn getting_through_a_turn_forgives_the_run() {
        let mut recovery = Recovery::default();
        let mut when = Instant::now();
        for _ in 0..SPIN_LIMIT - 1 {
            when += SPIN_WINDOW / 4;
            recovery.note(when);
        }
        assert_eq!(recovery.turn(|| "a whole turn, start to finish"), Some("a whole turn, start to finish"));
        when += SPIN_WINDOW / 4;
        recovery.note(when);
        assert!(!recovery.giving_up(), "the count starts again after a turn that worked");
    }

    /// A panic under a lock must not make that lock a second, permanent way
    /// to lose the session.
    #[test]
    fn a_poisoned_lock_still_opens() {
        use std::sync::Arc;
        let shared = Arc::new(Mutex::new(7));
        let holder = Arc::clone(&shared);
        let mut recovery = Recovery::default();
        recovery.turn(move || {
            let mut held = holder.lock_anyway();
            *held = 8;
            panic!("something went wrong with the lock held");
        });
        assert!(shared.lock().is_err(), "the lock really is poisoned");
        assert_eq!(*shared.lock_anyway(), 8, "and the compositor reads it anyway");
    }
}
