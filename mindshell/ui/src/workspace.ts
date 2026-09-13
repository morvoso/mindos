import * as actions from './actions';
import * as bridge from './bridge';
import { launchWithFeedback } from './app-match';
import { appearanceControls } from './appearance';
import { every, h, RESUME_EVENT } from './dom';
import { rectIn, setDesktopBar } from './geometry';
import { icon } from './icons';
import { modeInfo, perfRefresh, perfSubscribe, perfSwitch } from './perf';
import { systemReadout } from './readout';
import { openLibrary, resumeCard } from './resume';
import { spaceSwitch } from './space-switch';
import { activeSpace, followSpaces, spaces, updateSpace } from './spaces';
import { store } from './state';
import { systemControls } from './system-menu';
import { renderDesktopIcons } from './desktop-icons';
import type { Layout, PerfStatus, Space, WorkspaceShortcut } from './types';

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
    applyHome(false);
    requestAnimationFrame(() => { if (presenting) bridge.send('desktop.panel', { active: true }); });
  };
  let previousPrimary = false;
  let mounting = false;
  let again = false;
  // Tearing a workspace down writes to the layout (a pending note flushes on
  // the way out), and that change comes straight back here. Take the teardown
  // first, look again afterwards, and let a nested call ask for one more pass
  // instead of building a second workspace over the top of this one.
  function mount() {
    if (mounting) { again = true; return; }
    mounting = true;
    try {
      do {
        again = false;
        const primary = isMainOutput(output);
        if (primary === previousPrimary) return;
        const previous = dispose;
        dispose = undefined;
        previous?.();
        previousPrimary = primary;
        if (primary) dispose = mountDesktop();
      } while (again);
    } finally {
      mounting = false;
      // A workspace comes up shown; whether it belongs on the screen is
      // decided here, without a fade -- there was nothing to fade from.
      applyHome(false);
    }
  }
  function mountDesktop(): () => void {
    const offs: (() => void)[] = [];
    let alive = true;
    const appearance = appearanceControls(); const systemMenu = systemControls();
    offs.push(appearance.destroy, systemMenu.destroy);
    const title = h('span', { class: 'gaming-meta gaming-edition' });
    const header = h('header', { class: 'gaming-menubar' }, h('strong', { class: 'gaming-brand' }, h('i'), 'MINDOS'), title);
    const nav = h('nav', { class: 'gaming-nav', 'aria-label': 'Desktop shortcuts' });
    const main = h('section', { class: 'gaming-main productivity-main space-main', 'aria-label': 'Desktop' });
    const rail = h('aside', { class: 'gaming-rail', 'aria-label': 'System and notes' });
    const area = h('div', { class: 'gaming-workspace space-workspace' }, header, nav, main, rail);
    root.append(area);
    // The compositor's desk, the performance mode and the window layout follow
    // the active space; only the primary desktop drives them.
    offs.push(followSpaces());
    // The user's panels keep their edges whichever space the screen is on, so
    // the workspace lays itself out inside what they leave.
    const place = () => {
      const pads = { top: 0, right: 0, bottom: 80, left: 0 };
      for (const p of store.state.layout.panels) if (['*', 'primary', output].includes(p.output)) pads[p.edge] = Math.max(pads[p.edge], p.size + p.margin * 2);
      for (const edge of ['top', 'right', 'bottom', 'left'] as const) area.style.setProperty(`--gaming-${edge}`, `${pads[edge]}px`);
    };
    place(); offs.push(store.on('layout', place));
    // The crossfade between the home screen and the windows, and the only
    // thing that decides whether the workspace is on screen. The bar is not
    // part of it: it stays whichever view the screen is in, and the compositor
    // keeps windows out of the strip it covers (`reportBar` below). Everything
    // under it fades. `gaming-active` goes with the fade: what the workspace
    // covers while it is up -- the desktop icons, the desktop widgets -- is
    // what the user is meant to see once it steps aside, and `is-away` takes
    // the faded parts out of the layout rather than merely hiding them, which
    // is what stops whichever of the two is out of sight from sampling the
    // machine.
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
    const report = (e: unknown) => {
      if (!alive) return;
      let status = area.querySelector<HTMLElement>('.workspace-error');
      if (!status) { status = h('p', { class: 'workspace-error', role: 'status' }); area.append(status); }
      status.textContent = e instanceof Error || typeof e === 'string' ? String(e) : bridge.reason(e);
    };
    // Settings, the Game Library and Gaming Center are apps with windows of
    // their own; the desktop only opens them (or brings an open one forward).
    const openApp = (name: string, page?: string) => { void bridge.call('shell.openApp', { name, page }).catch(report); };
    const switcher = spaceSwitch(() => openApp('settings', 'shell'));
    offs.push(switcher.destroy);
    header.append(switcher.el, h('div', { class: 'header-actions' }, appearance.el, systemMenu.el));

    // What is on the desktop belongs to the space: the menu down the left,
    // the shortcuts, Resume playing and the notes. Moving to another space
    // fades that out and the new space's in; the bar and the switch stay.
    let body: (() => void) | undefined;
    let bodyKey = '';
    let shownSpace = '';
    let swaps: Animation[] = [];
    const build = (space: Space) => {
      body?.(); body = undefined;
      const run = (fn: () => Promise<unknown>) => () => { void fn().catch(report); };
      const link = (label: string, glyph: string, fn: () => void, page = '') => h('button', { class: 'gaming-nav-link', dataset: { page }, onclick: fn }, icon(glyph, 18), h('span', {}, label));
      const open = (name: string, page?: string) => () => openApp(name, page);
      const launch = (pattern: RegExp) => run(async () => {
        const app = store.state.apps.find(a => pattern.test(a.id));
        if (app) await bridge.call('apps.launch', { id: app.id });
        else openApp('settings', 'software');
      });
      const customNav = h('span', { class: 'workspace-custom-nav' });
      nav.replaceChildren(h('span', { class: 'gaming-meta' }, space.name),
        link('Home', space.icon || 'home', () => undefined, 'home'),
        ...(space.recent ? [link('Game Library', 'gamepad', run(openLibrary)), link('Gaming Center', 'sliders', open('gaming'), 'gaming')] : []),
        link('Files', 'folder', run(() => bridge.call('fs.open', { path: '~' }))),
        link('Browser', 'globe', launch(/firefox|chromium/i)),
        link('Settings', 'gear', open('settings'), 'settings'),
        customNav,
        h('span', { class: 'gaming-nav-bottom gaming-meta' }, 'Super + Space'));
      main.replaceChildren(); rail.replaceChildren();
      body = renderSpace(space.id, main, rail, output, customNav);
      area.dataset.space = space.id;
      title.textContent = `// ${space.name}`;
      nav.querySelector('[data-page=home]')?.setAttribute('aria-pressed', 'true');
    };
    const syncSpace = () => {
      const space = activeSpace();
      const key = JSON.stringify([space.id, space.name, space.icon, !!space.recent]);
      if (key === bodyKey) return;
      const switched = shownSpace !== '' && space.id !== shownSpace;
      // Set before building: the old body flushes its notes on the way out,
      // and that layout change comes straight back here.
      bodyKey = key; shownSpace = space.id;
      for (const a of swaps) a.cancel();
      swaps = [];
      const parts = [nav, main, rail];
      if (!switched || !shown || matchMedia('(prefers-reduced-motion: reduce)').matches) return build(space);
      swaps = parts.map(el => el.animate([{ opacity: 1, transform: 'none' }, { opacity: 0, transform: 'translateY(6px)' }], { duration: 110, easing: 'ease-in', fill: 'forwards' }));
      const outs = swaps;
      outs[0].onfinish = () => {
        if (!alive || swaps !== outs) return;
        const now = activeSpace();
        build(now.id === space.id ? now : space);
        for (const a of outs) a.cancel();
        swaps = parts.map(el => el.animate([{ opacity: 0, transform: 'translateY(-6px)' }, { opacity: 1, transform: 'none' }], { duration: 220, easing: 'ease-out' }));
      };
    };
    syncSpace();
    offs.push(store.on('layout', syncSpace));
    return () => { alive = false; for (const a of swaps) a.cancel(); body?.(); offs.forEach(off => off()); area.remove(); };
  }
  // The host asks for this when a window takes the keyboard.
  const offSink = bridge.on<{ active: boolean }>('desktop.present', p => present(p.active === true));
  // The desktop has finished fading out.
  const offAway = bridge.on('desktop.away', () => {
    if (presenting) return;
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
  mount();
  const offs = [store.on('outputs', () => mount()), store.on('prefs', () => mount()),
    store.on('windows', settleHome), store.on('editMode', () => applyHome()),
    offSink, offAway, offShortcut];
  return () => {
    if (settle !== undefined) clearTimeout(settle);
    dispose?.(); present(false); offs.forEach(off => off());
    window.removeEventListener('keydown', onKey);
  };
}

const LAUNCHERS = /^(steam|com\.valvesoftware\.Steam)\.desktop$|heroic|lutris|discord/i;
const OFFICE = /firefox|chromium|libreoffice-writer|libreoffice-calc|onlyoffice|thunderbird|evolution|geary/i;

/** The body of one space's desktop: Resume playing, its shortcuts and the
 *  files on the desktop in the middle; the machine and its notes on the right. */
function renderSpace(spaceId: string, main: HTMLElement, rail: HTMLElement, output: string, customNav: HTMLElement): () => void {
  let alive = true;
  const own = () => spaces().find(s => s.id === spaceId);
  const status = h('p', { class: 'play-status', role: 'status' });
  const say = (text: string) => { if (alive) status.textContent = text; };
  const run = (fn: () => Promise<unknown>) => () => { void fn().catch(e => say(bridge.reason(e))); };
  const card = (title: string, ...body: HTMLElement[]) => h('section', { class: 'gaming-rail-card work-card' }, h('header', { class: 'gaming-panel-title' }, h('h2', {}, title)), ...body);
  const open = (el: HTMLElement, appId: string) => run(async () => {
    if (!store.state.apps.some(a => a.id === appId)) return bridge.call('shell.openApp', { name: 'settings', page: 'software' });
    return launchWithFeedback(el, appId);
  });
  const glyph = (id: string) => /firefox|chromium/i.test(id) ? 'globe' : /mail|evolution|geary|thunderbird/i.test(id) ? 'mail' : /steam|heroic|lutris/i.test(id) ? 'gamepad' : /discord/i.test(id) ? 'headphones' : 'edit';
  // A space that has never had shortcuts starts with a few that suit it; once
  // the list is written down, even empty, it is the user's.
  const defaultShortcuts = (): WorkspaceShortcut[] => store.state.apps.filter(a => (own()?.recent ? LAUNCHERS : OFFICE).test(a.id)).slice(0, 4)
    .map(a => ({ id: `app-${a.id}`, appId: a.id, label: a.name, pinned: true, icon: glyph(a.id) }));
  const currentShortcuts = () => own()?.shortcuts ?? [];
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
  const resume = own()?.recent ? resumeCard(say) : undefined;
  const desktopFiles = h('div', { class: 'desktop-icons home-desktop-files' });
  main.append(...(resume ? [resume.el] : []), shortcutGrid, desktopFiles, status);
  const disposeFiles = renderDesktopIcons(main, desktopFiles, output);

  const notes = h('textarea', { class: 'work-notes', placeholder: 'Type a note…', 'aria-label': 'Notes for this space' });
  notes.value = own()?.notes ?? '';
  // Every keystroke lands in the local layout at once, so a space switch
  // carries it; the file write and the broadcast to every other page wait for
  // a pause in typing (or for this view to go away).
  const writeNotes = (l: Layout) => { const s = spaces(l).find(x => x.id === spaceId); if (s) s.notes = notes.value; };
  let pendingNotes: ReturnType<typeof setTimeout> | undefined;
  const flushNotes = () => {
    if (pendingNotes === undefined) return;
    clearTimeout(pendingNotes); pendingNotes = undefined;
    void store.updateLayout(writeNotes);
  };
  notes.addEventListener('input', () => {
    writeNotes(store.state.layout);
    if (pendingNotes !== undefined) clearTimeout(pendingNotes);
    pendingNotes = setTimeout(flushNotes, 400);
  });
  notes.addEventListener('blur', flushNotes);

  // The machine, on every space: the Task Manager in miniature and the
  // performance mode, which the space may have just set.
  const readout = systemReadout({ intervalMs: 3000 });
  const paused = h('p', { class: 'gaming-meta', hidden: true }, 'Sampling paused while gaming');
  const profile = h('div', { class: 'gaming-profiles' });
  const profileLabel = h('p', { class: 'gaming-meta' });
  let perf: PerfStatus | undefined, switching = false;
  const renderPerf = () => {
    profileLabel.textContent = perf ? `${modeInfo(perf.effective).label}${perf.game ? ' / GameMode active' : ' / System profile'}` : 'Performance controls unavailable';
    profile.replaceChildren(...(['quiet', 'balanced', 'performance'] as const).map((mode) => h('button', {
      class: 'btn', 'aria-pressed': String(perf?.mode === mode), disabled: switching || !perf,
      onclick: async () => {
        switching = true; renderPerf();
        try { say(await perfSwitch(mode)); }
        catch (e) { say(bridge.reason(e)); }
        finally { switching = false; if (alive) renderPerf(); }
      },
    }, mode === 'performance' ? 'Max' : modeInfo(mode).label)));
  };
  // A game gets the machine to itself: the readout's timer already stops on a
  // desktop surface while one runs (see quiet.ts), so the card only has to say
  // why the numbers have stopped moving.
  const gameState = () => { paused.hidden = !store.state.game; readout.el.hidden = !!store.state.game; };
  rail.append(card('System', h('div', { class: 'gaming-rail-body' }, paused, readout.el, profileLabel, profile)), card('Notes', notes));
  renderPerf(); gameState();
  renderShortcuts();
  const seed = () => {
    const s = own();
    if (!s || s.shortcuts !== undefined) return;
    const defaults = defaultShortcuts();
    if (defaults.length) void updateSpace(spaceId, x => { x.shortcuts ??= defaults; });
  };
  seed();
  let shortcutKey = JSON.stringify(currentShortcuts()) + activation();
  const offApps = store.on('apps', () => { seed(); shortcutKey = JSON.stringify(currentShortcuts()) + activation(); renderShortcuts(); });
  const offLayout = store.on('layout', () => {
    // Layout events carry every change from every page; only the shortcuts matter here.
    const key = JSON.stringify(currentShortcuts()) + activation();
    if (key === shortcutKey) return;
    shortcutKey = key; renderShortcuts();
    // Notes typed in another window (a second session of Settings, say).
    if (document.activeElement !== notes && pendingNotes === undefined) notes.value = own()?.notes ?? '';
  });
  const offs = [offApps, offLayout, store.on('game', gameState),
    perfSubscribe(rail, (s) => { perf = s; renderPerf(); }),
    every(rail, 15000, () => { if (!store.state.game && !document.hidden) void perfRefresh(); })];
  return () => { alive = false; flushNotes(); readout.destroy(); resume?.destroy(); disposeFiles(); offs.forEach(off => off()); };
}
