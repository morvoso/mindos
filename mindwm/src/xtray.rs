//! The XEmbed system tray host: legacy X11 tray icons (Wine's `Shell_NotifyIcon`,
//! old GTK2/Qt4 programs, Java) for a shell that only speaks StatusNotifier.
//!
//! The compositor owns the `_NET_SYSTEM_TRAY_S<n>` selection on a second
//! connection to its own XWayland. A program that wants a tray icon sends a
//! dock request naming its icon window; the icon is reparented into a small
//! override-redirect container parked off screen, mapped, and told it is
//! embedded. Every X11 toplevel is composite-redirected by the window manager,
//! so the container has a pixmap of its own and its pixels can be read back
//! with `GetImage` whether or not it is on screen. The pixels go to the shell
//! over the IPC socket as `tray` events; the shell draws them next to its
//! StatusNotifier items and sends clicks back, which are replayed on the icon
//! with XTest after the container is moved under the pointer, so the program
//! sees an ordinary click at the real cursor position.
//!
//! Nothing here depends on the window manager side of the compositor: the
//! host is a plain X client of XWayland, and the containers are ordinary
//! override-redirect windows the WM leaves alone.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::Serialize;
use tracing::{debug, info, warn};
use x11rb::connection::Connection;
use x11rb::protocol::composite::ConnectionExt as _;
use x11rb::protocol::xproto::*;
use x11rb::protocol::xtest::ConnectionExt as _;
use x11rb::protocol::Event;
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as _;
use x11rb::CURRENT_TIME;

use smithay::utils::x11rb::X11Source;

/// Icons are asked to be this big (logical X pixels).
pub const ICON_SIZE: u16 = 24;
/// Where containers wait while nobody clicks them.
const PARK: (i32, i32) = (-4 * ICON_SIZE as i32, -4 * ICON_SIZE as i32);
/// How long a container stays under the pointer after a click, so the
/// program's own popup (a menu) can find it there.
const CLICK_HOLD: Duration = Duration::from_millis(600);
/// How often the icons are read back.
pub const POLL: Duration = Duration::from_millis(400);

const SYSTEM_TRAY_REQUEST_DOCK: u32 = 0;
const XEMBED_EMBEDDED_NOTIFY: u32 = 0;
const XEMBED_VERSION: u32 = 0;
const XEMBED_MAPPED: u32 = 1;

x11rb::atom_manager! {
    pub Atoms: AtomsCookie {
        _NET_SYSTEM_TRAY_OPCODE,
        _NET_SYSTEM_TRAY_ORIENTATION,
        _NET_SYSTEM_TRAY_VISUAL,
        _NET_WM_NAME,
        _NET_WM_PID,
        _XEMBED,
        _XEMBED_INFO,
        MANAGER,
        UTF8_STRING,
        WM_STATE,
        _MINDWM_TRAY_CLOSE,
    }
}

/// What the shell gets for one icon: its pixels as RGBA, straight (not
/// premultiplied) alpha, row-major, base64 encoded.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TrayItem {
    /// The icon's X window id; stable for the life of the icon.
    pub id: u32,
    /// `_NET_WM_NAME` / `WM_NAME` of the icon window (Wine puts the tooltip there).
    pub title: String,
    /// `WM_CLASS` class of the icon window (Wine: the program's exe name).
    pub class: String,
    pub pid: Option<u32>,
    pub width: u16,
    pub height: u16,
    pub pixels: String,
}

struct Icon {
    container: Window,
    title: String,
    class: String,
    pid: Option<u32>,
    mapped: bool,
    /// RGBA, straight alpha, `ICON_SIZE` square.
    pixels: Option<Vec<u8>>,
    /// Set while the container sits under the pointer for a click.
    parked_again_at: Option<Instant>,
}

impl std::fmt::Debug for XTray {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("XTray")
            .field("selection", &self.selection)
            .field("icons", &self.icons.len())
            .finish()
    }
}

pub struct XTray {
    conn: Arc<RustConnection>,
    root: Window,
    atoms: Atoms,
    selection: Window,
    /// The 32-bit TrueColor visual advertised to icons, with its colormap.
    argb: Option<(Visualid, Colormap)>,
    lsb_first: bool,
    icons: BTreeMap<Window, Icon>,
    /// The published list changed since `snapshot` was last taken.
    dirty: bool,
}

impl XTray {
    /// Take the tray selection on display `:<display>`; returns the host and
    /// the calloop source that delivers its X events.
    pub fn start(display_number: u32) -> Result<(XTray, X11Source), String> {
        let (conn, screen_num) = x11rb::connect(Some(&format!(":{display_number}"))).map_err(|e| e.to_string())?;
        let conn = Arc::new(conn);
        let setup = conn.setup();
        let lsb_first = setup.image_byte_order == ImageOrder::LSB_FIRST;
        let screen = setup.roots[screen_num].clone();
        let root = screen.root;
        let atoms = Atoms::new(&*conn).map_err(|e| e.to_string())?.reply().map_err(|e| e.to_string())?;
        let selection_atom = conn
            .intern_atom(false, format!("_NET_SYSTEM_TRAY_S{screen_num}").as_bytes())
            .map_err(|e| e.to_string())?
            .reply()
            .map_err(|e| e.to_string())?
            .atom;

        conn.composite_query_version(0, 4).map_err(|e| e.to_string())?.reply().map_err(|e| e.to_string())?;

        let argb = screen
            .allowed_depths
            .iter()
            .filter(|d| d.depth == 32)
            .flat_map(|d| d.visuals.iter())
            .find(|v| v.class == VisualClass::TRUE_COLOR)
            .map(|v| v.visual_id)
            .and_then(|visual| {
                let cmap = conn.generate_id().ok()?;
                conn.create_colormap(ColormapAlloc::NONE, cmap, root, visual).ok()?.check().ok()?;
                Some((visual, cmap))
            });

        // The selection owner: an unmapped 1x1 window nobody sees.
        let selection = conn.generate_id().map_err(|e| e.to_string())?;
        let (depth, visual, aux) = match argb {
            Some((visual, cmap)) => (
                32,
                visual,
                CreateWindowAux::new().background_pixel(0).border_pixel(0).colormap(cmap),
            ),
            None => (x11rb::COPY_DEPTH_FROM_PARENT, x11rb::COPY_FROM_PARENT, CreateWindowAux::new()),
        };
        conn.create_window(depth, selection, root, 0, 0, 1, 1, 0, WindowClass::INPUT_OUTPUT, visual, &aux)
            .map_err(|e| e.to_string())?
            .check()
            .map_err(|e| e.to_string())?;
        conn.change_property32(PropMode::REPLACE, selection, atoms._NET_SYSTEM_TRAY_ORIENTATION, AtomEnum::CARDINAL, &[0])
            .map_err(|e| e.to_string())?;
        if let Some((visual, _)) = argb {
            conn.change_property32(PropMode::REPLACE, selection, atoms._NET_SYSTEM_TRAY_VISUAL, AtomEnum::VISUALID, &[visual])
                .map_err(|e| e.to_string())?;
        }
        conn.change_property8(PropMode::REPLACE, selection, atoms._NET_WM_NAME, atoms.UTF8_STRING, b"mindwm tray")
            .map_err(|e| e.to_string())?;

        conn.set_selection_owner(selection, selection_atom, CURRENT_TIME).map_err(|e| e.to_string())?;
        let owner = conn
            .get_selection_owner(selection_atom)
            .map_err(|e| e.to_string())?
            .reply()
            .map_err(|e| e.to_string())?
            .owner;
        if owner != selection {
            return Err("another client owns the system tray selection".into());
        }
        let manager = ClientMessageEvent::new(32, root, atoms.MANAGER, [CURRENT_TIME, selection_atom, selection, 0, 0]);
        conn.send_event(false, root, EventMask::STRUCTURE_NOTIFY, manager).map_err(|e| e.to_string())?;
        conn.flush().map_err(|e| e.to_string())?;
        info!(display_number, argb = argb.is_some(), "XEmbed system tray host up");

        let source = X11Source::new(conn.clone(), selection, atoms._MINDWM_TRAY_CLOSE);
        let tray = XTray {
            conn,
            root,
            atoms,
            selection,
            argb,
            lsb_first,
            icons: BTreeMap::new(),
            dirty: false,
        };
        Ok((tray, source))
    }

    /// True after anything the shell should hear about changed.
    pub fn take_dirty(&mut self) -> bool {
        std::mem::replace(&mut self.dirty, false)
    }

    pub fn is_empty(&self) -> bool {
        self.icons.is_empty()
    }

    /// ICCCM bookkeeping smithay skips: once a client has unmapped its own
    /// top-level window, WM_STATE must read Withdrawn. Wine waits for that
    /// before it treats the hide as finished, and a Windows program that
    /// hides a window and shows it again (a tray restore, a splash screen,
    /// most dialogs) otherwise never gets its window mapped again.
    pub fn withdraw(&self, window: Window) {
        let wm_state = self.atoms.WM_STATE;
        if self.conn.change_property32(PropMode::REPLACE, window, wm_state, wm_state, &[0, 0]).is_ok() {
            let _ = self.conn.flush();
        }
    }

    /// The published items, in docking order (icons that unmapped themselves
    /// or have not drawn yet are left out).
    pub fn snapshot(&self) -> Vec<TrayItem> {
        self.icons
            .iter()
            .filter(|(_, icon)| icon.mapped)
            .filter_map(|(&id, icon)| {
                let pixels = icon.pixels.as_ref()?;
                Some(TrayItem {
                    id,
                    title: icon.title.clone(),
                    class: icon.class.clone(),
                    pid: icon.pid,
                    width: ICON_SIZE,
                    height: ICON_SIZE,
                    pixels: base64(pixels),
                })
            })
            .collect()
    }

    pub fn handle_event(&mut self, event: Event) {
        match event {
            Event::ClientMessage(ev) if ev.window == self.selection && ev.type_ == self.atoms._NET_SYSTEM_TRAY_OPCODE => {
                let data = ev.data.as_data32();
                if data[1] == SYSTEM_TRAY_REQUEST_DOCK {
                    let icon = data[2];
                    if let Err(err) = self.dock(icon) {
                        warn!(icon, err, "tray icon could not be docked");
                    }
                }
            }
            Event::DestroyNotify(ev) => {
                if self.icons.contains_key(&ev.window) {
                    self.undock(ev.window, "destroyed");
                }
            }
            Event::ReparentNotify(ev) => {
                if let Some(icon) = self.icons.get(&ev.window) {
                    if ev.parent != icon.container {
                        self.undock(ev.window, "reparented away");
                    }
                }
            }
            Event::UnmapNotify(ev) => {
                if let Some(icon) = self.icons.get_mut(&ev.window) {
                    if icon.mapped {
                        icon.mapped = false;
                        self.dirty = true;
                    }
                }
            }
            Event::MapNotify(ev) => {
                if let Some(icon) = self.icons.get_mut(&ev.window) {
                    if !icon.mapped {
                        icon.mapped = true;
                        self.dirty = true;
                    }
                }
            }
            Event::PropertyNotify(ev) => {
                if self.icons.contains_key(&ev.window) {
                    if ev.atom == self.atoms._XEMBED_INFO {
                        self.apply_xembed_info(ev.window);
                    } else if ev.atom == self.atoms._NET_WM_NAME
                        || ev.atom == u32::from(AtomEnum::WM_NAME)
                        || ev.atom == u32::from(AtomEnum::WM_CLASS)
                    {
                        self.refresh_names(ev.window);
                    }
                }
            }
            Event::ConfigureNotify(ev) => {
                // Icons that resize themselves are put back to the tray size.
                if let Some(icon) = self.icons.get(&ev.window) {
                    if ev.width != ICON_SIZE || ev.height != ICON_SIZE || ev.x != 0 || ev.y != 0 {
                        let aux = ConfigureWindowAux::new().x(0).y(0).width(u32::from(ICON_SIZE)).height(u32::from(ICON_SIZE));
                        let _ = self.conn.configure_window(ev.window, &aux);
                        let _ = self.conn.flush();
                    }
                    let _ = icon;
                }
            }
            Event::Error(err) => debug!(?err, "tray host X error"),
            _ => {}
        }
    }

    fn dock(&mut self, icon: Window) -> Result<(), String> {
        if self.icons.contains_key(&icon) {
            return Ok(());
        }
        let conn = &self.conn;
        let attrs = conn.get_window_attributes(icon).map_err(|e| e.to_string())?.reply().map_err(|e| e.to_string())?;
        let geometry = conn.get_geometry(icon).map_err(|e| e.to_string())?.reply().map_err(|e| e.to_string())?;
        let depth = geometry.depth;
        let visual = attrs.visual;

        // The container shares the icon's visual, so a ParentRelative
        // background (what most icons use) is legal; a 32-bit one is
        // transparent, anything else sits on the panel colour.
        let cmap = match self.argb {
            Some((v, cmap)) if v == visual => cmap,
            _ => {
                let cmap = conn.generate_id().map_err(|e| e.to_string())?;
                conn.create_colormap(ColormapAlloc::NONE, cmap, self.root, visual)
                    .map_err(|e| e.to_string())?;
                cmap
            }
        };
        let background = if depth == 32 { 0 } else { 0x0010_151c };
        let container = conn.generate_id().map_err(|e| e.to_string())?;
        let aux = CreateWindowAux::new()
            .background_pixel(background)
            .border_pixel(0)
            .colormap(cmap)
            .override_redirect(1)
            .event_mask(EventMask::SUBSTRUCTURE_NOTIFY | EventMask::STRUCTURE_NOTIFY);
        conn.create_window(
            depth,
            container,
            self.root,
            PARK.0 as i16,
            PARK.1 as i16,
            ICON_SIZE,
            ICON_SIZE,
            0,
            WindowClass::INPUT_OUTPUT,
            visual,
            &aux,
        )
        .map_err(|e| e.to_string())?
        .check()
        .map_err(|e| e.to_string())?;

        let icon_mask = EventMask::STRUCTURE_NOTIFY | EventMask::PROPERTY_CHANGE;
        conn.change_window_attributes(icon, &ChangeWindowAttributesAux::new().event_mask(icon_mask))
            .map_err(|e| e.to_string())?;
        conn.reparent_window(icon, container, 0, 0).map_err(|e| e.to_string())?;
        let size = ConfigureWindowAux::new().width(u32::from(ICON_SIZE)).height(u32::from(ICON_SIZE));
        conn.configure_window(icon, &size).map_err(|e| e.to_string())?;
        conn.map_window(container).map_err(|e| e.to_string())?;
        let notify = ClientMessageEvent::new(
            32,
            icon,
            self.atoms._XEMBED,
            [CURRENT_TIME, XEMBED_EMBEDDED_NOTIFY, 0, container, XEMBED_VERSION],
        );
        conn.send_event(false, icon, EventMask::NO_EVENT, notify).map_err(|e| e.to_string())?;
        conn.flush().map_err(|e| e.to_string())?;

        self.icons.insert(
            icon,
            Icon {
                container,
                title: String::new(),
                class: String::new(),
                pid: None,
                mapped: false,
                pixels: None,
                parked_again_at: None,
            },
        );
        self.refresh_names(icon);
        self.apply_xembed_info(icon);
        info!(icon, container, depth, "tray icon docked");
        Ok(())
    }

    fn undock(&mut self, icon: Window, why: &str) {
        if let Some(entry) = self.icons.remove(&icon) {
            let _ = self.conn.destroy_window(entry.container);
            let _ = self.conn.flush();
            debug!(icon, why, "tray icon undocked");
            self.dirty = true;
        }
    }

    /// `_XEMBED_INFO` says whether the icon wants to be mapped; an icon
    /// without the property is mapped anyway (the spec's default).
    fn apply_xembed_info(&mut self, icon: Window) {
        let flags = self
            .conn
            .get_property(false, icon, self.atoms._XEMBED_INFO, self.atoms._XEMBED_INFO, 0, 2)
            .ok()
            .and_then(|c| c.reply().ok())
            .and_then(|r| r.value32().map(|v| v.collect::<Vec<u32>>()))
            .and_then(|v| v.get(1).copied())
            .unwrap_or(XEMBED_MAPPED);
        let want_mapped = flags & XEMBED_MAPPED != 0;
        let _ = if want_mapped { self.conn.map_window(icon) } else { self.conn.unmap_window(icon) };
        let _ = self.conn.flush();
    }

    fn refresh_names(&mut self, icon: Window) {
        let title = self
            .text_property(icon, self.atoms._NET_WM_NAME, self.atoms.UTF8_STRING)
            .or_else(|| self.text_property(icon, AtomEnum::WM_NAME.into(), AtomEnum::STRING.into()))
            .unwrap_or_default();
        let class = self
            .text_property(icon, AtomEnum::WM_CLASS.into(), AtomEnum::STRING.into())
            .map(|s| s.split('\0').filter(|p| !p.is_empty()).last().unwrap_or("").to_string())
            .unwrap_or_default();
        let pid = self
            .conn
            .get_property(false, icon, self.atoms._NET_WM_PID, AtomEnum::CARDINAL, 0, 1)
            .ok()
            .and_then(|c| c.reply().ok())
            .and_then(|r| r.value32().and_then(|mut v| v.next()));
        if let Some(entry) = self.icons.get_mut(&icon) {
            if entry.title != title || entry.class != class || entry.pid != pid {
                entry.title = title;
                entry.class = class;
                entry.pid = pid;
                self.dirty = true;
            }
        }
    }

    fn text_property(&self, window: Window, property: Atom, kind: Atom) -> Option<String> {
        let reply = self.conn.get_property(false, window, property, kind, 0, 1024).ok()?.reply().ok()?;
        if reply.format != 8 || reply.value.is_empty() {
            return None;
        }
        Some(String::from_utf8_lossy(&reply.value).trim_end_matches('\0').to_string())
    }

    /// Read every mapped icon back and park containers whose click hold is
    /// over. Called from a timer every `POLL`.
    pub fn poll(&mut self) {
        let now = Instant::now();
        let ids: Vec<Window> = self.icons.keys().copied().collect();
        for id in ids {
            let (container, mapped, park) = {
                let icon = &self.icons[&id];
                (
                    icon.container,
                    icon.mapped,
                    icon.parked_again_at.is_some_and(|t| now >= t),
                )
            };
            if park {
                let aux = ConfigureWindowAux::new().x(PARK.0).y(PARK.1);
                let _ = self.conn.configure_window(container, &aux);
                let _ = self.conn.flush();
                if let Some(icon) = self.icons.get_mut(&id) {
                    icon.parked_again_at = None;
                }
            }
            if !mapped {
                continue;
            }
            match self.capture(container) {
                Ok(pixels) => {
                    let icon = self.icons.get_mut(&id).unwrap();
                    if icon.pixels.as_ref() != Some(&pixels) {
                        icon.pixels = Some(pixels);
                        self.dirty = true;
                    }
                }
                Err(err) => debug!(icon = id, err, "tray icon read-back failed"),
            }
        }
    }

    /// The container's pixels as straight-alpha RGBA.
    fn capture(&self, container: Window) -> Result<Vec<u8>, String> {
        let conn = &self.conn;
        let pixmap = conn.generate_id().map_err(|e| e.to_string())?;
        conn.composite_name_window_pixmap(container, pixmap).map_err(|e| e.to_string())?;
        let image = conn
            .get_image(ImageFormat::Z_PIXMAP, pixmap, 0, 0, ICON_SIZE, ICON_SIZE, !0)
            .map_err(|e| e.to_string())
            .and_then(|c| c.reply().map_err(|e| e.to_string()));
        let _ = conn.free_pixmap(pixmap);
        let image = image?;
        Ok(to_rgba(&image.data, image.depth, self.lsb_first, usize::from(ICON_SIZE) * usize::from(ICON_SIZE)))
    }

    /// Replay a click on an icon: buttons 1/2/3, 4/5 for wheel up/down,
    /// 6/7 for wheel left/right. `pointer` is where the pointer is, in X
    /// (logical) coordinates.
    pub fn click(&mut self, id: u32, button: u8, pointer: (i32, i32)) -> Result<(), String> {
        let container = self.icons.get(&id).map(|i| i.container).ok_or_else(|| format!("no such tray icon: {id}"))?;
        if !(1..=7).contains(&button) {
            return Err(format!("no such button: {button}"));
        }
        let conn = &self.conn;
        let half = i32::from(ICON_SIZE / 2);
        let (px, py) = pointer;
        let aux = ConfigureWindowAux::new().x(px - half).y(py - half).stack_mode(StackMode::ABOVE);
        conn.configure_window(container, &aux).map_err(|e| e.to_string())?;
        conn.flush().map_err(|e| e.to_string())?;
        let (x, y) = (px.clamp(i16::MIN as i32, i16::MAX as i32) as i16, py.clamp(i16::MIN as i32, i16::MAX as i32) as i16);
        conn.xtest_fake_input(MOTION_NOTIFY_EVENT, 0, CURRENT_TIME, self.root, x, y, 0)
            .map_err(|e| e.to_string())?;
        conn.xtest_fake_input(BUTTON_PRESS_EVENT, button, CURRENT_TIME, self.root, 0, 0, 0)
            .map_err(|e| e.to_string())?;
        conn.xtest_fake_input(BUTTON_RELEASE_EVENT, button, CURRENT_TIME, self.root, 0, 0, 0)
            .map_err(|e| e.to_string())?;
        conn.flush().map_err(|e| e.to_string())?;
        if let Some(icon) = self.icons.get_mut(&id) {
            icon.parked_again_at = Some(Instant::now() + CLICK_HOLD);
        }
        debug!(id, button, px, py, "tray icon click replayed");
        Ok(())
    }
}

/// ZPixmap data of depth 24 or 32 to straight-alpha RGBA. Depth-32 windows
/// hold premultiplied ARGB (what XRender and every toolkit draw into them).
pub fn to_rgba(data: &[u8], depth: u8, lsb_first: bool, pixels: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(pixels * 4);
    for px in data.chunks_exact(4).take(pixels) {
        let (b, g, r, a) = if lsb_first {
            (px[0], px[1], px[2], px[3])
        } else {
            (px[3], px[2], px[1], px[0])
        };
        if depth == 32 {
            if a == 0 {
                out.extend_from_slice(&[0, 0, 0, 0]);
            } else {
                let un = |c: u8| ((u32::from(c) * 255 + u32::from(a) / 2) / u32::from(a)).min(255) as u8;
                out.extend_from_slice(&[un(r), un(g), un(b), a]);
            }
        } else {
            out.extend_from_slice(&[r, g, b, 255]);
        }
    }
    while out.len() < pixels * 4 {
        out.extend_from_slice(&[0, 0, 0, 0]);
    }
    out
}

/// Standard base64 with padding; enough for a few kilobytes of pixels.
pub fn base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity((bytes.len() + 2) / 3 * 4);
    for chunk in bytes.chunks(3) {
        let n = chunk.iter().enumerate().fold(0u32, |acc, (i, &b)| acc | (u32::from(b) << (16 - 8 * i)));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(TABLE[((n >> (18 - 6 * i)) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_pixels() {
        // premultiplied half-transparent red, then an opaque green, LSB first (B G R A)
        let data = [0, 0, 128, 128, 0, 255, 0, 255];
        assert_eq!(to_rgba(&data, 32, true, 2), vec![255, 0, 0, 128, 0, 255, 0, 255]);
        // depth 24 ignores the fourth byte
        assert_eq!(to_rgba(&[1, 2, 3, 9], 24, true, 1), vec![3, 2, 1, 255]);
        // MSB first is A R G B
        assert_eq!(to_rgba(&[255, 3, 2, 1], 32, false, 1), vec![3, 2, 1, 255]);
        // short data is padded with transparent pixels
        assert_eq!(to_rgba(&[], 32, true, 1), vec![0, 0, 0, 0]);
    }

    #[test]
    fn encodes_base64() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64(b"foobar"), "Zm9vYmFy");
        assert_eq!(base64(&[0xff, 0xfe, 0xfd]), "//79");
    }
}
