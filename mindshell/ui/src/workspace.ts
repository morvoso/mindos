import * as actions from './actions';
import * as bridge from './bridge';
import { launchWithFeedback } from './app-match';
import { appearanceControls } from './appearance';
import { renderSettings } from './apps/settings';
import { renderGaming } from './apps/gaming';
import { h, RESUME_EVENT } from './dom';
import { rectIn, setDesktopBar } from './geometry';
import { renderGamingDesktop } from './gaming-desktop';
import { icon } from './icons';
import { modeSwitch, type WorkspaceMode } from './mode-switch';
import { setPerfMode } from './perf';
import { systemReadout } from './readout';
import { store } from './state';
import { systemControls } from './system-menu';
import { renderDesktopIcons } from './desktop-icons';
import type { Layout, PerfMode, WorkspaceShortcut } from './types';

type Mode = WorkspaceMode;
type Page = { name: string; page?: string; arg?: string };
/// What each desktop mode asks of the machine: everything it has for gaming,
/// and back to the everyday profile for work.
const MODE_PERF: Record<Mode, PerfMode> = { gaming: 'performance', productivity: 'balanced' };
export function isMainOutput(output: string): boolean {
  const outputs = store.state.outputs;
  const primary = outputs.find(o => o.primary) ?? outputs.find(o => o.name === store.prefs.primary_output) ?? outputs[0];
  return !primary || !output || primary.name === output;
}

/** One workspace on the primary display. Secondary desktops only paint wallpaper. */
export function renderWorkspace(root: HTMLElement, output: string): () => void {
  let dispose: (() => void) | undefined;
  // The home screen is the ground the desktop stands on, not a permanent
  // fixture: with nothing open it is the whole screen, and the moment a window
  // opens it crossfades away and leaves the windows (over the wallpaper, the
  // desktop icons and any desktop widgets). Closing the last one brings it
  // back. Super+D asks for it over the top of the windows in between.
  //
  // Two different fades do that, and which one runs is the whole trick. At
  // ground level the surface never moves: only the workspace element's own
  // opacity changes, so there is no layer change, no repaint of the ground and
  // nothing for the compositor's startup screen to show through. Summoning it
  // over the windows is the surface's fade, run by the host in the same frame
  // as the layer change (`present_desktop`), and the workspace element is set
  // straight to full opacity underneath it -- fading both at once would show
  // the crossfade twice over.
  let presenting = false;
  /** A window is on this screen and would be covered by the home screen. */
  const occupied = () => store.state.windows.some(w => !w.minimized && isMainOutput(w.output ?? ''));
  /** Whether the home screen belongs on the screen right now. */
  const atHome = () => !store.state.editMode && (presenting || !occupied());
  /** Set by the mounted workspace: fade its view in or out. */
  let showWorkspace: ((on: boolean, animate: boolean) => void) | undefined;
  let reported: boolean | undefined;
  let settle: ReturnType<typeof setTimeout> | undefined;
  // The indicator in the panel is the only thing that can say which of the two
  // views the screen is in, since it is the only thing on screen in both.
  const applyHome = (animate = true) => {
    if (settle !== undefined) { clearTimeout(settle); settle = undefined; }
    const on = atHome();
    if (on !== reported) {
      reported = on;
      if (isMainOutput(output)) bridge.send('desktop.view', { home: on });
    }
    showWorkspace?.(on, animate);
  };
  // Coming back is worth a short wait; going away is not. An application that
  // replaces its own window -- a splash screen, a relaunch -- would otherwise
  // flash the home screen between the two.
  const settleHome = () => {
    const on = atHome();
    // Summoned over windows that have since closed: it is the ground again and
    // has nothing left to be in front of, so it stops being a summoned page.
    // Nothing moves on screen when that happens, only the layer under it.
    const grounded = presenting && !occupied();
    if (on === reported && !grounded) {
      if (settle !== undefined) { clearTimeout(settle); settle = undefined; }
      return;
    }
    if (!on) return applyHome();
    if (settle === undefined) settle = setTimeout(() => {
      settle = undefined;
      if (presenting && !occupied()) present(false);
      else applyHome();
    }, 180);
  };
  const present = (on: boolean) => {
    if (on === presenting || !isMainOutput(output)) return;
    // Nothing to summon it over: it is already the ground, and raising it
    // would only empty the layer underneath and flash.
    if (on && !occupied()) return applyHome();
    presenting = on;
    if (!on) {
      // The host fades the surface out; the workspace element keeps its
      // opacity until that is over (`desktop.away`), or the fade runs twice.
      // Only the indicator is told now, so it turns with the fade.
      if (atHome() !== reported) {
        reported = !reported;
        bridge.send('desktop.view', { home: reported });
      }
      // With windows behind it the surface fades out; with nothing behind it
      // the same page stays on screen either way, so it drops straight back to
      // the ground rather than fading out and reappearing.
      return bridge.send('desktop.panel', { active: false, fade: occupied() });
    }
    // Back to the page that was open before the desktop went away, and only
    // then ask for the raise: it fades in showing where the user left off.
    putAway?.(false);
    applyHome(false);
    requestAnimationFrame(() => { if (presenting) bridge.send('desktop.panel', { active: true }); });
  };
  let mountedMode: Mode | undefined;
  let openPage: ((p: Page) => void) | undefined;
  /// Show the home view instead of the open system page, or bring it back.
  let putAway: ((away: boolean) => void) | undefined;
  let previousPrimary = false;
  const mode = (): Mode => store.state.layout.desktop.workspace?.mode ?? 'gaming';
  /// Ask `mindos-perf` for the mode this workspace runs in. A game already
  /// running holds its own mode until it ends (GameMode's doing), so this is
  /// what the machine goes back to rather than something that fights it.
  const applyModePerf = (m: Mode) => {
    void setPerfMode(MODE_PERF[m]).catch((e) => console.warn('performance mode', e));
  };
  let mounting = false;
  let again = false;
  // Tearing a workspace down writes to the layout (a pending note flushes on
  // the way out), and that change comes straight back here. Take the teardown
  // first, read the mode again afterwards, and let a nested call ask for one
  // more pass instead of building a second workspace over the top of this one.
  function mount(animate = false) {
    if (mounting) { again = true; return; }
    mounting = true;
    try {
      do {
        again = false;
        const primary = isMainOutput(output);
        if (primary === previousPrimary && (!primary || mountedMode === mode())) return;
        const previous = dispose;
        const previousMode = mountedMode;
        dispose = undefined; openPage = undefined; mountedMode = undefined;
        previous?.();
        previousPrimary = primary;
        if (!primary) continue;
        mountedMode = mode();
        dispose = mountMode(mountedMode, animate, previousMode);
        // Switching the desktop between the two modes is also a statement
        // about what the machine is for, so the performance mode follows it.
        // Only on a real switch: a shell that restarts, or a display that
        // becomes the primary one, must not walk over a mode the user chose
        // by hand since.
        if (previousMode && previousMode !== mountedMode) applyModePerf(mountedMode);
      } while (again);
    } finally {
      mounting = false;
      // A workspace comes up shown; whether it belongs on the screen is
      // decided here, without a fade -- there was nothing to fade from.
      applyHome(false);
    }
  }
  function mountMode(current: Mode, animate: boolean, from?: Mode): () => void {
    const offs: (() => void)[] = [];
    let systemDispose: (() => void) | undefined;
    let alive = true;
    let productivityCustomNav: HTMLElement | undefined;
    let area: HTMLElement, nav: HTMLElement, main: HTMLElement, rail: HTMLElement, header: HTMLElement;
    if (current === 'gaming') {
      const desktop = renderGamingDesktop(root, output);
      offs.push(desktop.destroy);
      area = desktop.el;
      nav = area.querySelector('.gaming-nav')!;
      main = area.querySelector('.gaming-main')!;
      rail = area.querySelector('.gaming-rail')!;
      header = area.querySelector('.gaming-menubar')!;
    } else {
      const appearance = appearanceControls(); const systemMenu = systemControls(); offs.push(appearance.destroy, systemMenu.destroy);
      header = h('header', { class: 'gaming-menubar' }, h('strong', { class: 'gaming-brand' }, h('i'), 'MINDOS'), h('span', { class: 'gaming-meta gaming-edition' }, '// Home'), h('div', { class: 'header-actions' }, appearance.el, systemMenu.el));
      nav = h('nav', { class: 'gaming-nav', 'aria-label': 'Desktop shortcuts' });
      main = h('section', { class: 'gaming-main productivity-main' });
      rail = h('aside', { class: 'gaming-rail', 'aria-label': 'Work and home tools' });
      area = h('div', { class: 'gaming-workspace productivity-workspace' }, header, nav, main, rail);
      root.append(area);
      // The user's panels keep their edges whichever view the screen is in, so
      // the workspace lays itself out inside what they leave.
      const place = () => {
        const pads = { top: 0, right: 0, bottom: 80, left: 0 };
        for (const p of store.state.layout.panels) if (['*', 'primary', output].includes(p.output)) pads[p.edge] = Math.max(pads[p.edge], p.size + p.margin * 2);
        for (const edge of ['top', 'right', 'bottom', 'left'] as const) area.style.setProperty(`--gaming-${edge}`, `${pads[edge]}px`);
      };
      place(); offs.push(store.on('layout', place));
      const customNav = h('span', { class: 'workspace-custom-nav' });
      productivityCustomNav = customNav;
      offs.push(renderProductivity(main, rail, output, customNav));
      (area as HTMLElement).dataset.productivityNav = 'true';
    }
    area.dataset.workspaceMode = current;
    // The crossfade between the two views, and the only thing that decides
    // whether the workspace is on screen. The bar is not part of it: it stays
    // whichever view the screen is in, and the compositor keeps windows out
    // of the strip it covers (`reportBar` below). Everything under it fades.
    // `gaming-active` goes with the fade: what the workspace covers while it
    // is up -- the desktop icons, the desktop widgets -- is what the user is
    // meant to see once it steps aside, and `is-away` takes the faded parts
    // out of the layout rather than merely hiding them, which is what stops
    // whichever of the two is out of sight from sampling the machine.
    let shown = true;
    let fades: Animation[] = [];
    const stopFades = () => { for (const f of fades) f.cancel(); fades = []; };
    showWorkspace = (on, animate) => {
      root.classList.toggle('gaming-active', on);
      if (on === shown) return;
      shown = on;
      stopFades();
      if (on) {
        area.classList.remove('is-away');
        // Anything that stopped sampling while it was off screen catches up now.
        window.dispatchEvent(new Event(RESUME_EVENT));
      }
      const body = [...area.children].filter((el): el is HTMLElement => el !== header);
      if (!animate || matchMedia('(prefers-reduced-motion: reduce)').matches || !body.length) {
        area.classList.toggle('is-away', !on);
        return;
      }
      fades = body.map(el => el.animate([{ opacity: on ? 0 : 1 }, { opacity: on ? 1 : 0 }],
        { duration: on ? 220 : 130, easing: on ? 'ease-out' : 'ease-in', fill: 'forwards' }));
      fades[fades.length - 1].onfinish = () => {
        if (!on) area.classList.add('is-away');
        stopFades();
      };
    };
    offs.push(() => { showWorkspace = undefined; stopFades(); });
    // Nothing reserves room for the bar through layer-shell -- the desktop
    // window is anchored to every edge, so it would claim the whole screen --
    // and it is on screen even when the rest of the home screen is not. So
    // the compositor is told how far down the bar reaches, and keeps windows
    // below that. Only the primary screen has one; the others report nothing.
    let barSize = -1;
    const reportBar = () => {
      if (!alive) return;
      const size = isMainOutput(output) ? Math.round(header.getBoundingClientRect().bottom) : 0;
      // The desktop icons keep clear of the bar the same way the compositor
      // keeps the windows clear of it, so they are told at the same time.
      setDesktopBar(size);
      if (size === barSize) return;
      barSize = size;
      bridge.send('desktop.bar', { size });
    };
    const watchBar = new ResizeObserver(() => reportBar());
    // The bar's own height, and where the user's panels push it to.
    watchBar.observe(header); watchBar.observe(area);
    offs.push(() => watchBar.disconnect(), store.on('layout', reportBar));
    reportBar();
    if (animate && !matchMedia('(prefers-reduced-motion: reduce)').matches && !store.state.game) {
      area.animate([{ opacity: .35, transform: 'translateY(12px)' }, { opacity: 1, transform: 'translateY(0)' }], { duration: 240, easing: 'ease-out' });
    }
    const title = header.querySelector<HTMLElement>('.gaming-edition')!;
    const homeTitle = current === 'gaming' ? 'Library' : 'Home';
    const panelHost = h('section', { class: 'desktop-system-panel', hidden: true });
    const panelBody = h('div', { class: 'desktop-system-body' });
    const back = h('button', { class: 'btn', onclick: () => showHome() }, icon('arrow-left', 14), `Back to ${homeTitle}`);
    panelHost.append(h('header', { class: 'desktop-panel-toolbar' }, back), panelBody);
    area.append(panelHost);
    // The system page mounted in the panel, or undefined for the home view. It
    // stays mounted while the desktop is away behind the windows: what shows
    // through the gaps is the desktop, and summoning it again comes back to
    // the page the user was on rather than to the top of the menu.
    let openName: 'settings' | 'gaming' | undefined;
    const showPanel = (on: boolean) => {
      if (!openName) return;
      panelHost.hidden = !on; main.hidden = on; rail.hidden = on;
      area.classList.toggle('system-page-open', on);
      title.textContent = `// ${on ? (openName === 'settings' ? 'Settings' : 'Gaming Center') : homeTitle}`;
      const page = on ? openName : 'home';
      for (const b of nav.querySelectorAll('button')) b.setAttribute('aria-pressed', String(b.dataset.page === page));
    };
    putAway = (away) => showPanel(!away);
    const showHome = () => {
      // Back to the library or the home view -- still the desktop, still
      // forward. Only a window taking focus sends it back.
      systemDispose?.(); systemDispose = undefined; panelBody.replaceChildren();
      openName = undefined;
      panelHost.hidden = true; main.hidden = false; rail.hidden = false;
      area.classList.remove('system-page-open', 'library-hidden');
      title.textContent = `// ${homeTitle}`;
      for (const b of nav.querySelectorAll('button')) b.setAttribute('aria-pressed', String(b.dataset.page === 'home'));
    };
    openPage = (p) => {
      if (p.name === 'library') {
        present(true);
        if (current === 'productivity') {
          void store.updateLayout(l => { l.desktop.workspace = { ...l.desktop.workspace, mode: 'gaming', notes: l.desktop.workspace?.notes ?? '' }; });
        } else showHome();
        return;
      }
      if (p.name !== 'settings' && p.name !== 'gaming') return;
      systemDispose?.(); panelBody.replaceChildren();
      const content = h('div', { class: `app-window embedded-app`, dataset: { app: p.name } });
      panelBody.append(content);
      openName = p.name;
      area.classList.remove('library-hidden');
      showPanel(true);
      present(true);
      systemDispose = p.name === 'settings' ? renderSettings(content, p.page) : renderGaming(content, p.page, p.arg);
    };
    const report = (e: unknown) => {
      if (!alive) return;
      let status = area.querySelector<HTMLElement>('.workspace-error');
      if (!status) { status = h('p', { class: 'workspace-error', role: 'status' }); area.append(status); }
      status.textContent = String(e);
    };
    const run = (fn: () => Promise<unknown>) => () => { void fn().catch(report); };
    const link = (label: string, glyph: string, fn: () => void, page = '') => h('button', { class: 'gaming-nav-link', dataset: { page }, onclick: fn }, icon(glyph, 18), h('span', {}, label));
    const open = (name: string, page?: string) => () => openPage?.({ name, page });
    const launch = (pattern: RegExp) => run(async () => {
      const app = store.state.apps.find(a => pattern.test(a.id));
      if (app) await bridge.call('apps.launch', { id: app.id });
      else openPage?.({ name: 'settings', page: 'software' });
    });
    nav.replaceChildren(h('span', { class: 'gaming-meta' }, current === 'gaming' ? 'Gaming' : 'Work'),
      link(homeTitle, current === 'gaming' ? 'gamepad' : 'grid', showHome, 'home'),
      ...(current === 'gaming' ? [link('Gaming Center', 'sliders', open('gaming'), 'gaming')] : [
        link('Documents', 'folder', run(() => bridge.call('fs.open', { path: '~/Documents' }))),
      ]),
      ...(current === 'gaming' ? [link('Files', 'folder', run(() => bridge.call('fs.open', { path: '~' }))), link('Browser', 'globe', launch(/firefox|chromium/i))] : []),
      link('Settings', 'gear', open('settings'), 'settings'),
      ...(current === 'productivity' && productivityCustomNav ? [productivityCustomNav] : []),
      h('span', { class: 'gaming-nav-bottom gaming-meta' }, 'Super + Space'));
    const modes = modeSwitch(current, m => {
      if (m !== mode()) void store.updateLayout(l => { l.desktop.workspace = { ...l.desktop.workspace, mode: m, notes: l.desktop.workspace?.notes ?? '' }; });
    }, animate ? from : undefined);
    header.insertBefore(modes, header.querySelector('.header-actions'));
    showHome();
    return () => { alive = false; systemDispose?.(); offs.forEach(off => off()); area.remove(); };
  }
  const offOpen = bridge.on<Page>('desktop.open', p => { if (isMainOutput(output)) { store.setEditMode(false); openPage?.(p); } });
  // The host asks for this when a window takes the keyboard.
  const offSink = bridge.on<{ active: boolean }>('desktop.present', p => present(p.active === true));
  // The desktop has finished fading out. Put the open page away now that
  // nothing can see it happen, so what shows between the windows is the
  // desktop rather than a settings page nobody is looking at.
  const offAway = bridge.on('desktop.away', () => {
    if (presenting) return;
    // Only when the windows have the screen: with nothing open the home screen
    // is still being looked at, and closing its open page would be a jump.
    if (occupied()) putAway?.(true);
    // The surface is invisible at this point, so the workspace can go back to
    // whatever the ground calls for without a fade of its own.
    applyHome(false);
  });
  // Super+D from the compositor: forward if it is behind, behind if it is up.
  const offShortcut = store.on('shortcut', () => { if (store.lastShortcut === 'desktop') present(!presenting); });
  const onKey = (e: KeyboardEvent) => {
    if (e.key === 'Escape' && presenting && !e.defaultPrevented) present(false);
  };
  window.addEventListener('keydown', onKey);
  const localOpen = (e: Event) => { if (isMainOutput(output)) openPage?.((e as CustomEvent<Page>).detail); };
  const appTitle = (e: Event) => {
    const el = root.querySelector('.gaming-edition');
    if (el && root.querySelector('.system-page-open')) el.textContent = `// ${(e as CustomEvent<string>).detail.replace(' · ', ' / ')}`;
  };
  window.addEventListener('desktop.open', localOpen); window.addEventListener('desktop.title', appTitle);
  mount();
  const offs = [store.on('layout', () => mount(true)), store.on('outputs', () => mount()), store.on('prefs', () => mount()),
    store.on('windows', settleHome), store.on('editMode', () => applyHome()),
    offOpen, offSink, offAway, offShortcut];
  return () => {
    if (settle !== undefined) clearTimeout(settle);
    dispose?.(); present(false); offs.forEach(off => off());
    window.removeEventListener('keydown', onKey);
    window.removeEventListener('desktop.open', localOpen); window.removeEventListener('desktop.title', appTitle);
  };
}

function renderProductivity(main: HTMLElement, rail: HTMLElement, output: string, customNav: HTMLElement): () => void {
  let alive = true;
  const status = h('p', { class: 'play-status', role: 'status' });
  const run = (fn: () => Promise<unknown>) => () => { void fn().catch(e => { if (alive) status.textContent = String(e); }); };
  const card = (title: string, ...body: HTMLElement[]) => h('section', { class: 'gaming-rail-card work-card' }, h('header', { class: 'gaming-panel-title' }, h('h2', {}, title)), ...body);
  const open = (el: HTMLElement, appId: string) => run(async () => {
    if (!store.state.apps.some(a => a.id === appId)) return bridge.call('shell.openApp', { name: 'settings', page: 'software' });
    return launchWithFeedback(el, appId);
  });
  const defaultShortcuts = (): WorkspaceShortcut[] => store.state.apps.filter(a => /firefox|chromium|libreoffice-writer|libreoffice-calc|onlyoffice|thunderbird|evolution|geary/i.test(a.id)).slice(0, 4).map(a => ({ id: `app-${a.id}`, appId: a.id, label: a.name, pinned: true, icon: /firefox|chromium/i.test(a.id) ? 'globe' : /mail|evolution|geary/i.test(a.id) ? 'mail' : 'edit' }));
  const currentShortcuts = () => store.state.layout.desktop.workspace?.shortcuts ?? defaultShortcuts();
  const saveShortcuts = (shortcuts: WorkspaceShortcut[]) => void store.updateLayout(l => { l.desktop.workspace = { ...l.desktop.workspace, mode: 'productivity', notes: l.desktop.workspace?.notes ?? '', shortcuts }; });
  const shortcutGrid = h('div', { class: 'work-shortcuts' });
  const addButton = h('button', { class: 'work-shortcut-add', title: 'Add an application shortcut' }, h('span', { class: 'work-shortcut-art' }, icon('plus', 26)), h('span', {}, 'Add shortcut'));
  addButton.addEventListener('click', () => {
    const r = rectIn(main.closest<HTMLElement>('.win') ?? document.body, addButton);
    actions.openPopup('app-picker', {}, { keyboard: true, anchor: { x: r.x, y: r.y, w: r.w, h: r.h, edge: 'top' } });
  });

  /** Single or double click to open, chosen in Settings; the menu is always single. */
  const activation = () => store.state.layout.desktop.workspace?.activate ?? 'single';

  const shortcutTile = (s: WorkspaceShortcut) => {
    const app = store.state.apps.find(a => a.id === s.appId);
    const art = app?.icon
      ? h('img', { src: app.icon, alt: '', width: 48, height: 48, onerror: (e: Event) => (e.target as HTMLElement).replaceWith(icon(s.icon || 'box', 48)) })
      : icon(s.icon || 'box', 48);
    const el = h('button', { class: `work-shortcut${s.pinned ? ' pinned' : ''}`, 'aria-label': `Open ${app?.name || s.label}` },
      h('span', { class: 'work-shortcut-art' }, art, h('span', { class: 'work-shortcut-spin' })),
      h('span', { class: 'work-shortcut-name' }, app?.name || s.label));
    const launch = open(el, s.appId);
    el.addEventListener(activation() === 'double' ? 'dblclick' : 'click', launch);
    el.addEventListener('contextmenu', ev => {
      ev.preventDefault();
      const root = main.closest<HTMLElement>('.win') ?? document.body;
      const b = root.getBoundingClientRect();
      const scale = b.width / root.offsetWidth || 1;
      actions.openPopup('context-menu', {
        title: app?.name || s.label,
        items: [
          { label: 'Open', icon: 'window', action: { call: 'apps.launch', params: { id: s.appId } } },
          { label: s.pinned ? 'Unpin from the menu' : 'Pin to the menu', icon: 'pin', action: { shortcut: { id: s.id, op: 'pin' } } },
          { label: '', separator: true },
          { label: 'Remove shortcut', icon: 'trash', danger: true, action: { shortcut: { id: s.id, op: 'remove' } } },
          // Removing the shortcut leaves the app installed; this removes the app.
          ...(app ? [{ label: `Uninstall ${app.name}`, icon: 'trash', danger: true, action: { call: 'apps.uninstall', params: { id: s.appId } } }] : []),
        ],
        anchor: { x: (ev.clientX - b.left) / scale, y: (ev.clientY - b.top) / scale, w: 0, h: 0 },
      });
    });
    return el;
  };

  const renderShortcuts = () => {
    const shortcuts = currentShortcuts();
    shortcutGrid.replaceChildren(...shortcuts.map(shortcutTile), addButton);
    // Only pinned shortcuts belong in the left menu; the desktop keeps them all.
    customNav.replaceChildren(...shortcuts.filter(s => s.pinned).map(s => {
      const el = h('button', { class: 'gaming-nav-link', dataset: { shortcut: s.id } }, icon(s.icon || 'box', 18), h('span', {}, store.state.apps.find(a => a.id === s.appId)?.name || s.label));
      el.addEventListener('click', open(el, s.appId));
      return el;
    }));
  };
  main.setAttribute('aria-label', 'Desktop');
  const desktopFiles = h('div', { class: 'desktop-icons home-desktop-files' });
  main.append(shortcutGrid, desktopFiles, status);
  const disposeFiles = renderDesktopIcons(main, desktopFiles, output);
  const notes = h('textarea', { class: 'work-notes', placeholder: 'Type a note…', 'aria-label': 'Desktop notes', value: store.state.layout.desktop.workspace?.notes ?? '' });
  notes.value = store.state.layout.desktop.workspace?.notes ?? '';
  // Every keystroke lands in the local layout at once, so a mode switch carries
  // it; the file write and the broadcast to every other page wait for a pause
  // in typing (or for this view to go away).
  const workspaceNotes = (l: Layout) => { l.desktop.workspace = { ...l.desktop.workspace, mode: l.desktop.workspace?.mode ?? 'productivity', notes: notes.value }; };
  let pendingNotes: ReturnType<typeof setTimeout> | undefined;
  const flushNotes = () => {
    if (pendingNotes === undefined) return;
    clearTimeout(pendingNotes); pendingNotes = undefined;
    void store.updateLayout(workspaceNotes);
  };
  notes.addEventListener('input', () => {
    workspaceNotes(store.state.layout);
    if (pendingNotes !== undefined) clearTimeout(pendingNotes);
    pendingNotes = setTimeout(flushNotes, 400);
  });
  notes.addEventListener('blur', flushNotes);
  // The same readout the gaming rail carries, so the machine is visible in
  // either mode without either one sampling it twice.
  const readout = systemReadout({ intervalMs: 3000 });
  rail.append(card('System', h('div', { class: 'gaming-rail-body' }, readout.el)), card('Notes', notes));
  renderShortcuts();
  // The first-run suggestions are only a suggestion until they are written
  // down; the right-click menu and the picker both edit the saved list.
  const seed = () => {
    if (store.state.layout.desktop.workspace?.shortcuts?.length) return;
    const defaults = defaultShortcuts();
    if (defaults.length) saveShortcuts(defaults);
  };
  seed();
  let shortcutKey = JSON.stringify(currentShortcuts()) + activation();
  const offApps = store.on('apps', () => { seed(); shortcutKey = JSON.stringify(currentShortcuts()) + activation(); renderShortcuts(); });
  const offLayout = store.on('layout', () => {
    // Layout events carry every change from every page; only the shortcuts matter here.
    const key = JSON.stringify(currentShortcuts()) + activation();
    if (key === shortcutKey) return;
    shortcutKey = key; renderShortcuts();
  });
  return () => { alive = false; flushNotes(); readout.destroy(); disposeFiles(); offApps(); offLayout(); };
}
