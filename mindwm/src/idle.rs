//! Idling: the screensaver, the automatic lock and switching the displays off.
//!
//! The compositor keeps the clock, because it is the only process that sees
//! every key, click and gesture, and it enforces the lock: while the session
//! is locked nothing but the shell's lock windows is drawn or reachable. The
//! shell draws the screensaver and the lock screen, and asks to unlock once
//! the password checks out.
//!
//! The timings live in the preferences (`Prefs::idle`), so the Settings app
//! changes them with the ordinary `set_prefs` request and they survive a
//! restart. Everything is counted from the last input event:
//!
//! ```text
//!  input ────────────── screensaver ────── lock ────── displays off
//!         idle.screensaver      idle.lock       idle.blank   (seconds, 0 = never)
//! ```
//!
//! `zwp_idle_inhibit` (a video player, a full-screen game) and the shell's
//! `inhibit_idle` request hold the whole sequence off while
//! `idle.stay_awake_when_busy` is set.

use std::collections::HashSet;
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use smithay::input::pointer::CursorImageStatus;
use smithay::reexports::calloop::timer::{TimeoutAction, Timer};
use smithay::reexports::calloop::RegistrationToken;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::wayland::idle_inhibit::IdleInhibitHandler;
use smithay::wayland::idle_notify::{IdleNotifierHandler, IdleNotifierState};

use crate::state::{AnvilState, Backend};

/// The layer-shell namespace the shell gives its lock windows
/// (`mindshell/src/windows.rs`). Surfaces with this namespace are the only
/// ones drawn while the session is locked.
pub const LOCK_NAMESPACE: &str = "mindshell-lock";

/// How far into idling the session has gone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Stage {
    /// Somebody is using the machine.
    #[default]
    Active,
    /// The screensaver is up; the next input event takes it away.
    Screensaver,
    /// The displays are switched off.
    Blank,
}

impl Stage {
    pub fn name(self) -> &'static str {
        match self {
            Stage::Active => "active",
            Stage::Screensaver => "screensaver",
            Stage::Blank => "blank",
        }
    }
}

#[derive(Debug, Default)]
pub struct IdleState {
    pub stage: Stage,
    pub locked: bool,
    /// When the last input event arrived.
    pub since: Option<Instant>,
    /// Surfaces holding a `zwp_idle_inhibitor_v1`.
    pub inhibitors: HashSet<WlSurface>,
    /// IPC clients asking to stay awake (the shell, while a game runs).
    pub holds: HashSet<u64>,
    timer: Option<RegistrationToken>,
    /// The last `idle` event sent, so nothing is repeated.
    last_event: Option<Value>,
}

impl<BackendData: Backend + 'static> AnvilState<BackendData> {
    /// Something asked the session to stay awake.
    pub fn idle_inhibited(&self) -> bool {
        self.prefs.idle.stay_awake_when_busy
            && (!self.idle.inhibitors.is_empty() || !self.idle.holds.is_empty())
    }

    /// How long the machine has been left alone.
    fn idle_for(&self) -> Duration {
        self.idle.since.map(|t| t.elapsed()).unwrap_or_default()
    }

    /// Called for every input event, before it reaches a client. `true` means
    /// the event only woke the screen up and must go no further: it took the
    /// screensaver away or lit the displays again, and nobody wants that key
    /// typed into the window (or the password field) underneath.
    pub fn note_activity(&mut self) -> bool {
        let woke = self.idle.stage != Stage::Active;
        self.idle.since = Some(Instant::now());
        self.idle_notifier.notify_activity(&self.seat);
        if woke {
            self.set_stage(Stage::Active);
        }
        self.arm_idle_timer();
        woke
    }

    /// Move to a stage, switching the displays off or on as it goes.
    fn set_stage(&mut self, stage: Stage) {
        if self.idle.stage == stage {
            return;
        }
        let was_blank = self.idle.stage == Stage::Blank;
        self.idle.stage = stage;
        // No pointer over a screensaver. The cursor is the compositor's to
        // draw, and a stationary one would sit in the same place for hours:
        // CSS in the shell's lock window cannot help, because a client only
        // gets to choose a cursor when the pointer moves, and moving it is
        // exactly what takes the screensaver away again. The next motion
        // after the wake-up puts the right cursor back.
        self.cursor_status = if stage == Stage::Active {
            CursorImageStatus::default_named()
        } else {
            CursorImageStatus::Hidden
        };
        if stage == Stage::Blank {
            tracing::info!("idle: switching the displays off");
            BackendData::set_blanked(self, true);
        } else if was_blank {
            tracing::info!("idle: switching the displays back on");
            BackendData::set_blanked(self, false);
        }
        self.idle_changed();
    }

    /// Lock (or unlock) the session. Locking hides every window: from here on
    /// only the shell's lock surfaces are drawn and only they get input.
    pub fn set_locked(&mut self, locked: bool) {
        if self.idle.locked == locked {
            return;
        }
        tracing::info!(locked, "the session is now {}", if locked { "locked" } else { "unlocked" });
        self.idle.locked = locked;
        if locked {
            self.window_cycle.finish();
            self.media_keys.stop();
            self.mindbar.clear_osd();
            // Nothing behind the lock screen may keep the keyboard.
            if let Some(keyboard) = self.seat.get_keyboard() {
                let serial = smithay::utils::SERIAL_COUNTER.next_serial();
                keyboard.set_focus(self, None, serial);
            }
            // Drop mouse grabs and focus as well: relative events otherwise
            // continue reaching a game until the next ordinary pointer motion.
            let pointer = self.pointer.clone();
            let serial = smithay::utils::SERIAL_COUNTER.next_serial();
            let time = self.clock.now().as_millis() as u32;
            pointer.unset_grab(self, serial, time);
            pointer.motion(self, None, &smithay::input::pointer::MotionEvent {
                location: pointer.current_location(), serial, time,
            });
            pointer.frame(self);
            self.mindbar.close();
        }
        self.idle.since = Some(Instant::now());
        self.idle_changed();
        self.arm_idle_timer();
    }

    /// Wake the screen without touching the lock: what `Super` does from the
    /// shell, and what the Settings app's *Wake* button does.
    pub fn wake_idle(&mut self) {
        self.idle.since = Some(Instant::now());
        self.set_stage(Stage::Active);
        self.arm_idle_timer();
    }

    /// Switch the displays off now (the Settings app's button).
    pub fn blank_now(&mut self) {
        if self.prefs.idle.lock_on_blank {
            self.set_locked(true);
        }
        self.set_stage(Stage::Blank);
        self.arm_idle_timer();
    }

    /// A client holds the session awake (`inhibit_idle`, or an IPC client
    /// that went away).
    pub fn set_idle_hold(&mut self, client: u64, on: bool) {
        let changed = if on {
            self.idle.holds.insert(client)
        } else {
            self.idle.holds.remove(&client)
        };
        if changed {
            self.idle_inhibit_changed();
        }
    }

    /// The set of things holding the session awake changed.
    pub fn idle_inhibit_changed(&mut self) {
        let inhibited = self.idle_inhibited();
        self.idle_notifier.set_is_inhibited(inhibited);
        if !inhibited {
            // Start counting from now, not from whenever the video started.
            self.idle.since = Some(Instant::now());
        }
        self.arm_idle_timer();
        self.idle_changed();
    }

    /// How long until the next thing is due to happen.
    fn next_deadline(&self) -> Option<Duration> {
        let s = &self.prefs.idle;
        let idle = self.idle_for();
        let mut next: Option<Duration> = None;
        let mut consider = |seconds: u32, already_done: bool| {
            if seconds == 0 || already_done {
                return;
            }
            let at = Duration::from_secs(seconds as u64);
            let left = at.saturating_sub(idle);
            next = Some(next.map_or(left, |n: Duration| n.min(left)));
        };
        consider(s.screensaver, self.idle.stage != Stage::Active);
        consider(s.lock, self.idle.locked);
        consider(s.blank, self.idle.stage == Stage::Blank);
        next
    }

    /// Arm one timer for the next stage boundary (nothing is armed while the
    /// session is held awake, or when every timeout is *never*).
    pub fn arm_idle_timer(&mut self) {
        if let Some(token) = self.idle.timer.take() {
            self.handle.remove(token);
        }
        if self.idle_inhibited() {
            return;
        }
        let Some(left) = self.next_deadline() else {
            return;
        };
        // A second of slack keeps a stage from being missed by rounding.
        let timer = Timer::from_duration(left.max(Duration::from_millis(200)));
        match self.handle.insert_source(timer, |_, _, state| {
            state.idle.timer = None;
            state.idle_tick();
            TimeoutAction::Drop
        }) {
            Ok(token) => self.idle.timer = Some(token),
            Err(err) => tracing::warn!(%err, "cannot arm the idle timer"),
        }
    }

    /// A deadline came up: move the session on as far as it should go.
    fn idle_tick(&mut self) {
        if self.idle_inhibited() {
            self.idle.since = Some(Instant::now());
            self.arm_idle_timer();
            return;
        }
        let s = self.prefs.idle.clone();
        let idle = self.idle_for().as_secs() as u32;
        let due = |seconds: u32| seconds > 0 && idle + 1 >= seconds;

        if due(s.lock) {
            self.set_locked(true);
        }
        if due(s.blank) {
            if s.lock_on_blank {
                self.set_locked(true);
            }
            self.set_stage(Stage::Blank);
        } else if due(s.screensaver) && self.idle.stage == Stage::Active {
            self.set_stage(Stage::Screensaver);
        }
        self.arm_idle_timer();
    }

    /// The `idle` event: where the session stands, for the shell.
    pub fn idle_json(&self) -> Value {
        json!({
            "stage": self.idle.stage.name(),
            "locked": self.idle.locked,
            "inhibited": self.idle_inhibited(),
            "saver": self.prefs.idle.saver,
        })
    }

    /// Tell the shell, unless it already knows.
    pub fn idle_changed(&mut self) {
        let payload = self.idle_json();
        if self.idle.last_event.as_ref() == Some(&payload) {
            return;
        }
        self.idle.last_event = Some(payload.clone());
        let mut event = payload;
        event["event"] = json!("idle");
        self.ipc_broadcast(&event.to_string());
    }
}

impl<BackendData: Backend + 'static> IdleNotifierHandler for AnvilState<BackendData> {
    fn idle_notifier_state(&mut self) -> &mut IdleNotifierState<Self> {
        &mut self.idle_notifier
    }
}

impl<BackendData: Backend + 'static> IdleInhibitHandler for AnvilState<BackendData> {
    fn inhibit(&mut self, surface: WlSurface) {
        self.idle.inhibitors.insert(surface);
        self.idle_inhibit_changed();
    }

    fn uninhibit(&mut self, surface: WlSurface) {
        self.idle.inhibitors.remove(&surface);
        self.idle_inhibit_changed();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prefs::IdleSettings;

    #[test]
    fn the_defaults_are_in_order() {
        let s = IdleSettings::default();
        assert!(s.screensaver < s.lock, "the screensaver comes before the lock");
        assert!(s.blank >= s.lock, "the displays go off no sooner than the lock");
        assert_eq!(s.saver, "shuffle");
    }

    #[test]
    fn stage_names_round_trip() {
        assert_eq!(Stage::default(), Stage::Active);
        assert_eq!(Stage::Active.name(), "active");
        assert_eq!(Stage::Screensaver.name(), "screensaver");
        assert_eq!(Stage::Blank.name(), "blank");
    }
}
