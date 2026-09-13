// Copyright (c) 2026 Black Arrow Software, LLC. All rights reserved.
// Licensed under the MindOS Software License; see LICENSE at the repository root.

//! Desks: the shell's spaces, as the compositor sees them.
//!
//! Every window belongs to one desk, a name the shell chooses ("gaming",
//! "work"). Only the windows of the current desk are in the space; the rest
//! are stashed the way a minimised window is -- out of the space, so not
//! drawn, not reachable and sent no frame callbacks -- and come back where
//! they were when their desk does. Stashing and minimising are independent:
//! a window minimised on one desk is still minimised when that desk comes
//! back.
//!
//! Some applications belong on every desk (a chat client, a music player).
//! The shell names them in the `sticky` list by app id; a sticky window is
//! never stashed, and travels with the desk: it belongs to whichever desk was
//! switched to last, so taking it off the list leaves it where the user is.
//! A dialog goes with the window it belongs to.
//!
//! The shell switches with `set_desk` (docs/SHELL.md). Anything that reaches
//! for a window on another desk -- a taskbar click, an activation request, a
//! second launch raising the first -- switches to that desk, and says so with
//! a `desk` event so the shell follows.

use std::cell::RefCell;
use std::collections::HashMap;

use smithay::utils::IsAlive;

use crate::{
    layout::LayoutMode,
    shell::WindowElement,
    state::{AnvilState, Backend, Minimized},
};

/// The current desk, the applications on every desk and the windows put away.
#[derive(Debug, Default)]
pub struct Desks {
    /// Empty until the shell first sets one; every window shows then.
    pub current: String,
    /// App ids shown on every desk (compared without regard to case).
    pub sticky: Vec<String>,
    /// Windows of other desks, bottom of the stack first.
    pub stashed: Vec<Minimized>,
    /// The window that had the keyboard when each desk was last left.
    last_focus: HashMap<String, u64>,
}

/// The desk a window belongs to; absent until the window is first seen.
#[derive(Debug, Default)]
struct DeskTag(RefCell<Option<String>>);

/// Whether `app_id` is on the sticky list.
pub fn is_sticky(sticky: &[String], app_id: &str) -> bool {
    !app_id.is_empty() && sticky.iter().any(|s| s.eq_ignore_ascii_case(app_id))
}

/// Whether a window of `desk` belongs on screen while `current` is the desk.
/// A window that has no desk yet (from before the shell set one) shows.
pub fn shows_on(current: &str, desk: &str, sticky: bool) -> bool {
    sticky || desk.is_empty() || desk == current
}

/// The sticky list as the shell sent it, without blanks or repeats.
pub fn clean_sticky(list: Vec<String>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for entry in list {
        let entry = entry.trim();
        if !entry.is_empty() && !out.iter().any(|e| e.eq_ignore_ascii_case(entry)) {
            out.push(entry.to_string());
        }
    }
    out
}

/// A switch to desk `new`, for one window: the desk it belongs to
/// afterwards and whether it shows.
///
/// A window with no desk adopts the new one, and so does a sticky one (it
/// travels with the user); everything else keeps its desk and shows only if
/// that is the new desk or it is sticky under the new list.
pub fn switch_window(new: &str, desk: &str, sticky_before: bool, sticky_after: bool) -> (String, bool) {
    let desk = if desk.is_empty() || sticky_before { new } else { desk };
    (desk.to_string(), shows_on(new, desk, sticky_after))
}

impl<BackendData: Backend + 'static> AnvilState<BackendData> {
    /// The desk a window belongs to ("" before it has one).
    pub fn window_desk(&self, window: &WindowElement) -> String {
        window
            .user_data()
            .get::<DeskTag>()
            .and_then(|tag| tag.0.borrow().clone())
            .unwrap_or_default()
    }

    fn has_desk(window: &WindowElement) -> bool {
        window.user_data().get::<DeskTag>().is_some_and(|tag| tag.0.borrow().is_some())
    }

    /// A window that has a desk, and not `desk` (without copying the name).
    fn elsewhere(window: &WindowElement, desk: &str) -> bool {
        window
            .user_data()
            .get::<DeskTag>()
            .is_some_and(|tag| tag.0.borrow().as_deref().is_some_and(|d| !d.is_empty() && d != desk))
    }

    fn set_window_desk(window: &WindowElement, desk: &str) {
        window.user_data().insert_if_missing(DeskTag::default);
        if let Some(tag) = window.user_data().get::<DeskTag>() {
            *tag.0.borrow_mut() = Some(desk.to_string());
        }
    }

    /// Every window the compositor manages: in the space, stashed, minimised.
    fn all_windows(&self) -> Vec<WindowElement> {
        self.space
            .elements()
            .cloned()
            .chain(self.desks.stashed.iter().map(|s| s.window.clone()))
            .chain(self.minimized.iter().map(|m| m.window.clone()))
            .collect()
    }

    /// The window a dialog belongs to, wherever that window is.
    pub fn window_parent(&self, window: &WindowElement) -> Option<WindowElement> {
        let mut candidates = self
            .space
            .elements()
            .chain(self.desks.stashed.iter().map(|s| &s.window))
            .chain(self.minimized.iter().map(|m| &m.window));
        if let Some(parent) = window.0.toplevel().and_then(|t| t.parent()) {
            return candidates.find(|w| w.wl_surface().as_deref() == Some(&parent)).cloned();
        }
        #[cfg(feature = "xwayland")]
        if let Some(parent) = window.0.x11_surface().and_then(|s| s.is_transient_for()) {
            return candidates
                .find(|w| matches!(w.0.x11_surface(), Some(s) if s.window_id() == parent))
                .cloned();
        }
        None
    }

    /// Whether a window shows on every desk: its app is on the sticky list,
    /// or it is a dialog of one that is.
    pub fn window_sticky(&self, window: &WindowElement) -> bool {
        self.sticky_under(&self.desks.sticky, window)
    }

    fn sticky_under(&self, sticky: &[String], window: &WindowElement) -> bool {
        if sticky.is_empty() {
            return false;
        }
        let mut window = window.clone();
        // Dialogs of dialogs; a cycle cannot be made, but a bound costs nothing.
        for _ in 0..8 {
            if is_sticky(sticky, &window.app_id()) {
                return true;
            }
            match self.window_parent(&window) {
                Some(parent) => window = parent,
                None => return false,
            }
        }
        false
    }

    pub fn is_stashed(&self, window: &WindowElement) -> bool {
        self.desks.stashed.iter().any(|s| &s.window == window)
    }

    /// Whether a window is hidden because its desk is not the current one
    /// (a minimised window counts when its desk would not show it either).
    pub fn is_away(&self, window: &WindowElement) -> bool {
        if self.is_stashed(window) {
            return true;
        }
        self.is_minimized(window)
            && !shows_on(&self.desks.current, &self.window_desk(window), self.window_sticky(window))
    }

    /// Give windows that have no desk one: the desk of the window they
    /// belong to, else the current desk.
    fn tag_new_windows(&mut self) {
        // Every turn asks; almost every turn the answer is no.
        let untagged = self
            .space
            .elements()
            .chain(self.desks.stashed.iter().map(|s| &s.window))
            .chain(self.minimized.iter().map(|m| &m.window))
            .any(|w| !Self::has_desk(w));
        if !untagged {
            return;
        }
        for window in self.all_windows() {
            if Self::has_desk(&window) {
                continue;
            }
            let desk = self
                .window_parent(&window)
                .filter(Self::has_desk)
                .map(|parent| self.window_desk(&parent))
                .unwrap_or_else(|| self.desks.current.clone());
            Self::set_window_desk(&window, &desk);
        }
    }

    /// Once per event-loop turn: forget stashed windows that are gone, give
    /// new windows a desk, and put away a window that opened for a desk that
    /// is not showing (a dialog of a window on another desk).
    pub fn desks_refresh(&mut self) {
        self.desks.stashed.retain(|s| s.window.alive());
        self.tag_new_windows();
        let strays: Vec<WindowElement> = self
            .space
            .elements()
            .filter(|w| Self::elsewhere(w, &self.desks.current))
            // Not before it has drawn: an xdg window is sent its first
            // configure from the space.
            .filter(|w| {
                let size = w.0.geometry().size;
                size.w > 0 && size.h > 0
            })
            .filter(|w| !self.window_sticky(w))
            .cloned()
            .collect();
        if strays.is_empty() {
            return;
        }
        for window in strays {
            self.stash_window(&window);
        }
        self.layout.dirty = true;
        self.refresh_focus();
    }

    /// Take a window out of the space until its desk comes back.
    fn stash_window(&mut self, window: &WindowElement) {
        if self.is_stashed(window) {
            return;
        }
        if let Some(entry) = self.take_out_of_space(window) {
            self.desks.stashed.push(entry);
        }
    }

    /// Put a stashed window back where it was, without raising the keyboard.
    fn restore_window(&mut self, window: &WindowElement) {
        let Some(pos) = self.desks.stashed.iter().position(|s| &s.window == window) else {
            return;
        };
        let entry = self.desks.stashed.remove(pos);
        self.put_back_in_space(&entry);
    }

    /// Switch to `desk`. `sticky` replaces the list of applications on every
    /// desk when given; `mode` is the layout mode the desk uses.
    pub fn set_desk(&mut self, desk: &str, sticky: Option<Vec<String>>, mode: Option<LayoutMode>) {
        self.desks.stashed.retain(|s| s.window.alive());
        self.tag_new_windows();
        let old = std::mem::replace(&mut self.desks.current, desk.to_string());
        let old_sticky = self.desks.sticky.clone();
        let changed = old != desk || sticky.as_ref().is_some_and(|s| clean_sticky(s.clone()) != old_sticky);
        if let Some(sticky) = sticky {
            self.desks.sticky = clean_sticky(sticky);
        }
        if old != desk {
            if let Some(focused) = self.focused_window() {
                if !old.is_empty() {
                    self.desks.last_focus.insert(old.clone(), focused.id());
                }
            }
        }

        // Decide for every window before moving any: stickiness follows
        // dialogs to their parents, which the moves would not change, but
        // the parents have to be found first.
        let mut stash = Vec::new();
        let mut restore = Vec::new();
        for window in self.all_windows() {
            let before = self.sticky_under(&old_sticky, &window);
            let after = self.window_sticky(&window);
            let (tag, shows) = switch_window(desk, &self.window_desk(&window), before, after);
            Self::set_window_desk(&window, &tag);
            if self.is_stashed(&window) {
                if shows {
                    restore.push(window);
                }
            } else if !shows && !self.is_minimized(&window) {
                stash.push(window);
            }
        }

        if !stash.is_empty() || !restore.is_empty() {
            // Tiles come back in the order they had; floating windows in the
            // stacking order (which is what `stash` is already in).
            if self.layout.mode.is_tiling() {
                let place: HashMap<u64, (String, usize)> = self
                    .layout
                    .outputs
                    .iter()
                    .flat_map(|(name, t)| t.order.iter().enumerate().map(move |(i, w)| (w.id(), (name.clone(), i))))
                    .collect();
                stash.sort_by_key(|w| place.get(&w.id()).cloned().map_or((1, String::new(), 0), |(n, i)| (0, n, i)));
            }
            for window in &stash {
                self.stash_window(window);
            }
            let restore: Vec<WindowElement> = self
                .desks
                .stashed
                .iter()
                .map(|s| s.window.clone())
                .filter(|w| restore.contains(w))
                .collect();
            for window in &restore {
                self.restore_window(window);
            }
            self.layout.dirty = true;
        }
        if let Some(mode) = mode {
            self.set_layout_mode(mode);
        }

        if old != desk {
            let back = self
                .desks
                .last_focus
                .get(desk)
                .and_then(|id| self.space.elements().find(|w| w.id() == *id).cloned());
            match back {
                Some(window) => self.activate_window(&window),
                None => {
                    // A window of this desk rather than one that is on every desk.
                    let top = self
                        .space
                        .elements()
                        .rev()
                        .find(|w| self.window_desk(w) == desk && !self.window_sticky(w))
                        .cloned();
                    match top {
                        Some(window) => self.activate_window(&window),
                        None => self.refresh_focus(),
                    }
                }
            }
        }
        self.request_repaint();
        if changed {
            tracing::info!(desk, sticky = ?self.desks.sticky, "desk");
            let line = crate::ipc::desk_event(&self.desks.current, &self.desks.sticky);
            self.ipc_broadcast(&line);
        }
    }

    /// Give a window to another desk: it goes away if that desk is not
    /// showing, and comes back if it is.
    pub fn move_window_to_desk(&mut self, window: &WindowElement, desk: &str) {
        self.tag_new_windows();
        Self::set_window_desk(window, desk);
        let shows = shows_on(&self.desks.current, desk, self.window_sticky(window));
        if self.is_stashed(window) {
            if shows {
                self.restore_window(window);
                self.layout.dirty = true;
            }
        } else if !shows && !self.is_minimized(window) {
            self.stash_window(window);
            self.layout.dirty = true;
            self.refresh_focus();
        }
        self.request_repaint();
    }

    /// Something reached for a window on another desk: go to that desk (the
    /// layout mode and the sticky list stay as they are; the shell follows
    /// the `desk` event with its own settings).
    pub fn show_desk_of(&mut self, window: &WindowElement) {
        if !self.is_away(window) {
            return;
        }
        let desk = self.window_desk(window);
        if desk != self.desks.current {
            self.set_desk(&desk, None, None);
        }
        // Stashed although its desk is showing: cannot happen through
        // `set_desk`, but a window must never be left unreachable.
        if self.is_stashed(window) {
            self.restore_window(window);
            self.layout.dirty = true;
        }
    }

    /// Hand a stashed window over to the minimised list (it was minimised
    /// while away, from the taskbar).
    pub fn minimize_stashed(&mut self, window: &WindowElement) -> bool {
        let Some(pos) = self.desks.stashed.iter().position(|s| &s.window == window) else {
            return false;
        };
        let entry = self.desks.stashed.remove(pos);
        self.minimized.push(entry);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn list(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn sticky_matches_app_ids_without_case() {
        let sticky = list(&["discord", "Vesktop"]);
        assert!(is_sticky(&sticky, "Discord"));
        assert!(is_sticky(&sticky, "vesktop"));
        assert!(!is_sticky(&sticky, "discord-canary"));
        assert!(!is_sticky(&sticky, ""));
        assert!(!is_sticky(&[], "discord"));
    }

    #[test]
    fn cleans_the_sticky_list() {
        assert_eq!(clean_sticky(list(&[" discord ", "", "Discord", "steam"])), list(&["discord", "steam"]));
    }

    #[test]
    fn windows_show_on_their_own_desk() {
        assert!(shows_on("work", "work", false));
        assert!(!shows_on("work", "gaming", false));
        assert!(shows_on("work", "gaming", true));
        // From before the shell chose a desk.
        assert!(shows_on("work", "", false));
    }

    #[test]
    fn a_switch_keeps_windows_on_their_desks() {
        // A game on the gaming desk goes away when work is chosen...
        assert_eq!(switch_window("work", "gaming", false, false), ("gaming".into(), false));
        // ...and an editor on the work desk comes back.
        assert_eq!(switch_window("work", "work", false, false), ("work".into(), true));
    }

    #[test]
    fn windows_without_a_desk_adopt_the_first_one() {
        assert_eq!(switch_window("gaming", "", false, false), ("gaming".into(), true));
    }

    #[test]
    fn sticky_windows_travel_with_the_user() {
        assert_eq!(switch_window("work", "gaming", true, true), ("work".into(), true));
        // Taken off the list while switching: it stays on the desk switched to.
        assert_eq!(switch_window("work", "gaming", true, false), ("work".into(), true));
        // Put on the list while switching: it shows, and belongs where it was.
        assert_eq!(switch_window("work", "gaming", false, true), ("gaming".into(), true));
    }
}
