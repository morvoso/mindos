// Copyright (c) 2026 Black Arrow Software, LLC. All rights reserved.
// Licensed under the MindOS Software License; see LICENSE at the repository root.

//! The X properties the window manager reads for itself.
//!
//! Smithay's XWayland window manager owns `_NET_WM_STATE` once it manages a
//! window, and never reads what the client put there. But a program that
//! wants to *start* fullscreen sets that property before it maps the window
//! -- Wine and Proton do it for every game that opens straight into the game,
//! SDL does it, and EWMH says the window manager must honour it -- so without
//! reading it the game comes up as an ordinary window with the dock over it.
//!
//! Two more properties decide whether a window that has just appeared may
//! take the keyboard: `WM_PROTOCOLS` (with WM_HINTS' input flag, ICCCM's four
//! input models) and `_NET_WM_USER_TIME`, whose zero means "map me, but do
//! not focus me". Neither reaches a Smithay compositor either.
//!
//! Reading them needs no window-manager privileges: this is a plain second
//! client connection to the same XWayland, opened when XWayland comes up.

use tracing::debug;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{AtomEnum, ConnectionExt as _, Window};
use x11rb::rust_connection::RustConnection;

x11rb::atom_manager! {
    Atoms: AtomsCookie {
        _NET_WM_STATE,
        _NET_WM_STATE_FULLSCREEN,
        _NET_WM_USER_TIME,
        WM_PROTOCOLS,
        WM_TAKE_FOCUS,
    }
}

/// What a window asked for before anyone mapped it.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct MapState {
    /// `_NET_WM_STATE` already contains `_NET_WM_STATE_FULLSCREEN`.
    pub fullscreen: bool,
    /// `_NET_WM_USER_TIME` is 0: the client asks not to be given the keyboard.
    pub no_focus: bool,
    /// `WM_PROTOCOLS` contains `WM_TAKE_FOCUS`: the client takes the focus
    /// itself when told to, whatever WM_HINTS' input flag says.
    pub take_focus: bool,
}

pub struct XProps {
    conn: RustConnection,
    atoms: Atoms,
    /// The bits of a resource id that name the client that created it. Two
    /// windows with the same value came from the same X connection.
    client_mask: u32,
}

impl XProps {
    /// Connect to `:<display>` as an ordinary client.
    pub fn start(display: u32) -> Option<Self> {
        let (conn, _) = x11rb::connect(Some(&format!(":{display}")))
            .map_err(|err| debug!(%err, "no second connection to XWayland: window state is the WM's own"))
            .ok()?;
        let client_mask = !conn.setup().resource_id_mask;
        let atoms = Atoms::new(&conn).ok()?.reply().ok()?;
        Some(XProps { conn, atoms, client_mask })
    }

    /// Read the three properties of `window` that the map request depends on.
    /// A window that has already gone away reads as all-false.
    pub fn at_map(&self, window: Window) -> MapState {
        MapState {
            fullscreen: self
                .atoms32(window, self.atoms._NET_WM_STATE, AtomEnum::ATOM.into())
                .is_some_and(|v| v.contains(&self.atoms._NET_WM_STATE_FULLSCREEN)),
            no_focus: self
                .atoms32(window, self.atoms._NET_WM_USER_TIME, AtomEnum::CARDINAL.into())
                .is_some_and(|v| v.first() == Some(&0)),
            take_focus: self
                .atoms32(window, self.atoms.WM_PROTOCOLS, AtomEnum::ATOM.into())
                .is_some_and(|v| v.contains(&self.atoms.WM_TAKE_FOCUS)),
        }
    }

    /// Whether two windows were created by the same X client connection --
    /// the game and its own dialogs, as against the launcher that started it
    /// or the tray helper that came with it.
    pub fn same_client(&self, a: Window, b: Window) -> bool {
        a & self.client_mask == b & self.client_mask
    }

    fn atoms32(&self, window: Window, property: u32, kind: u32) -> Option<Vec<u32>> {
        let reply = self
            .conn
            .get_property(false, window, property, kind, 0, 64)
            .ok()?
            .reply()
            .ok()?;
        reply.value32().map(|v| v.collect())
    }
}

impl std::fmt::Debug for XProps {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("XProps").finish()
    }
}
