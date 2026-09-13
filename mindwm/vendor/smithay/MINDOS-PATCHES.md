# Smithay 0.7.0, as MindOS carries it

This is the crates.io release of Smithay 0.7.0, used by `mindwm` through
`[patch.crates-io]` in `mindwm/Cargo.toml`. Everything here is upstream's
except the changes below. Drop the copy once an upgrade of Smithay brings the
same behaviour; the lock changes have no upstream equivalent and have to be
carried forward.

`diff -r` against `~/.cargo/registry/src/index.crates.io-*/smithay-0.7.0`
shows all of them.

## `_NET_ACTIVE_WINDOW` names the focused window

Upstream 0.7.0 wrote the window of every `FocusIn` *and* `FocusOut` event to
`_NET_ACTIVE_WINDOW`. Client windows never selected focus events, so the
events came from the root and the property always named the root. Wine reads
this property to decide which of its windows is in the foreground, and gives a
window that is not neither keyboard input nor its cursor clip: games ignored
the keyboard and let the pointer run off onto the next monitor.

- Client windows select `FOCUS_CHANGE`; `FocusIn` on a managed window
  (not grabs, not pointer focus) publishes it, `FocusOut` publishes nothing.
- `X11Wm::set_active_window` lets the compositor publish its keyboard focus,
  or no window when the focus is on a Wayland client.
- A `_NET_ACTIVE_WINDOW` client message reaches the compositor as
  `XwmHandler::active_window_request` (a no-op by default).

Upstream Smithay later made `FocusIn` ignore windows it does not manage and
added a similar handler method.

## `_NET_WM_STATE` set before mapping counts

EWMH lets a client put `_NET_WM_STATE_FULLSCREEN` (or the maximised states) on
a window before it maps. 0.7.0 never read the property, so `is_fullscreen()`
was false for a game opening in borderless fullscreen. `MapRequest` now reads it
(`X11Surface::update_net_state`) just before `map_window_request`.

## Locks taken twice on one thread panic instead of hanging

Smithay calls compositor code while holding some of its own locks:

- every surface-tree walk (`with_surface_tree_*`) holds the lock of the
  surface it is visiting while the closures run, and `with_states` holds it
  for its closure;
- `PointerHandle`, `KeyboardHandle` and `TouchHandle` hold their internal lock
  while grabs and focus targets (`enter`, `leave`, `motion`, ...) run;
- `layer_map_for_output` hands out the output's layer-map lock.

Code in one of those callbacks that takes the same lock again used to block
on itself with a std `Mutex`: `with_states` or `with_renderer_surface_state`
on the surface being walked, `current_location()` inside a pointer grab, a
second `layer_map_for_output` for the same output. The event loop stopped with
it and every display froze. On 2026-09-12 it took a hard reboot: mindwm's
`get_surfaces` report called `with_renderer_surface_state` inside a tree walk.

- `src/utils/checked_mutex.rs` adds `CheckedMutex`, a std `Mutex` that records
  which thread holds it. Locking it again from that thread panics with the
  type it guards; mindwm catches panics on its loop, so this costs one
  callback. It ignores poisoning for the same reason: after a caught panic,
  the locks it unwound through must still open. `lock()` still returns a
  `LockResult` (never an error) so the existing `.lock().unwrap()` calls stand.
- `SurfaceUserData::inner` (`wayland/compositor/handlers.rs`, `tree.rs`),
  `PointerHandle::inner`, `KbdRc::internal`, `TouchHandle::inner` and the
  `LayerMap` in the output's user data are `CheckedMutex`es.
  `PrivateSurfaceData::lock_user_data` checks first so its panic names the
  surface. `layer_map_for_output` returns a `CheckedMutexGuard`.
- `with_grab` in `input/{pointer,keyboard,touch}/mod.rs` catches a panic from
  the grab or focus callback, ends the grab (`GrabStatus::None`) and resumes
  the panic. Upstream left the grab `GrabStatus::Borrowed`, so every later
  event on that device panicked with "Accessed a pointer grab from within a
  pointer grab access" until mindwm's spin guard ended the session.
