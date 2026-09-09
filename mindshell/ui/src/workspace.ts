import * as actions from './actions';
import * as bridge from './bridge';
import { launchWithFeedback } from './app-match';
import { appearanceControls } from './appearance';
import { renderSettings } from './apps/settings';
import { renderGaming } from './apps/gaming';
import { h } from './dom';
import { rectIn } from './geometry';
import { renderGamingDesktop } from './gaming-desktop';
import { icon } from './icons';
import { store } from './state';
import { systemControls } from './system-menu';
import { renderDesktopIcons } from './desktop-icons';
import type { Layout, WorkspaceShortcut } from './types';

type Mode = 'gaming' | 'productivity';
type Page = { name: string; page?: string; arg?: string };
export function isMainOutput(output: string): boolean {
  const outputs = store.state.outputs;
  const primary = outputs.find(o => o.primary) ?? outputs.find(o => o.name === store.prefs.primary_output) ?? outputs[0];
  return !primary || !output || primary.name === output;
}

/** One workspace on the primary display. Secondary desktops only paint wallpaper. */
export function renderWorkspace(root: HTMLElement, output: string): () => void {
  let dispose: (() => void) | undefined;
  let mountedMode: Mode | undefined;
  let openPage: ((p: Page) => void) | undefined;
  let previousPrimary = false;
  const mode = (): Mode => store.state.layout.desktop.workspace?.mode ?? 'gaming';
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
        dispose = undefined; openPage = undefined; mountedMode = undefined;
        previous?.();
        previousPrimary = primary;
        if (!primary) continue;
        mountedMode = mode();
        dispose = mountMode(mountedMode, animate);
      } while (again);
    } finally {
      mounting = false;
    }
  }
  function mountMode(current: Mode, animate: boolean): () => void {
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
      const place = () => {
        area.hidden = store.state.editMode;
        root.classList.toggle('gaming-active', !store.state.editMode);
        const pads = { top: 0, right: 0, bottom: 80, left: 0 };
        for (const p of store.state.layout.panels) if (['*', 'primary', output].includes(p.output)) pads[p.edge] = Math.max(pads[p.edge], p.size + p.margin * 2);
        for (const edge of ['top', 'right', 'bottom', 'left'] as const) area.style.setProperty(`--gaming-${edge}`, `${pads[edge]}px`);
      };
      place(); offs.push(store.on('layout', place), store.on('editMode', place));
      const customNav = h('span', { class: 'workspace-custom-nav' });
      productivityCustomNav = customNav;
      offs.push(renderProductivity(main, rail, output, customNav));
      (area as HTMLElement).dataset.productivityNav = 'true';
    }
    area.dataset.workspaceMode = current;
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
    const showHome = () => {
      systemDispose?.(); systemDispose = undefined; panelBody.replaceChildren();
      bridge.send('desktop.panel', { active: false });
      panelHost.hidden = true; main.hidden = false; rail.hidden = false;
      area.classList.remove('system-page-open', 'library-hidden');
      title.textContent = `// ${homeTitle}`;
      for (const b of nav.querySelectorAll('button')) b.setAttribute('aria-pressed', String(b.dataset.page === 'home'));
    };
    openPage = (p) => {
      if (p.name === 'library') {
        if (current === 'productivity') {
          void store.updateLayout(l => { l.desktop.workspace = { ...l.desktop.workspace, mode: 'gaming', notes: l.desktop.workspace?.notes ?? '' }; });
        } else showHome();
        return;
      }
      if (!['settings', 'gaming'].includes(p.name)) return;
      systemDispose?.(); panelBody.replaceChildren();
      const content = h('div', { class: `app-window embedded-app`, dataset: { app: p.name } });
      panelBody.append(content); main.hidden = true; rail.hidden = true; panelHost.hidden = false;
      bridge.send('desktop.panel', { active: true });
      area.classList.add('system-page-open'); area.classList.remove('library-hidden');
      title.textContent = `// ${p.name === 'settings' ? 'Settings' : 'Gaming Center'}`;
      systemDispose = p.name === 'settings' ? renderSettings(content, p.page) : renderGaming(content, p.page, p.arg);
      for (const b of nav.querySelectorAll('button')) b.setAttribute('aria-pressed', String(b.dataset.page === p.name));
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
    nav.replaceChildren(h('span', { class: 'gaming-meta' }, current === 'gaming' ? 'Gaming' : 'Productivity'),
      link(homeTitle, current === 'gaming' ? 'gamepad' : 'grid', showHome, 'home'),
      ...(current === 'gaming' ? [link('Gaming Center', 'sliders', open('gaming'), 'gaming')] : [
        link('Documents', 'folder', run(() => bridge.call('fs.open', { path: '~/Documents' }))),
      ]),
      ...(current === 'gaming' ? [link('Files', 'folder', run(() => bridge.call('fs.open', { path: '~' }))), link('Browser', 'globe', launch(/firefox|chromium/i))] : []),
      link('Settings', 'gear', open('settings'), 'settings'),
      ...(current === 'productivity' && productivityCustomNav ? [productivityCustomNav] : []),
      h('span', { class: 'gaming-nav-bottom gaming-meta' }, 'Super + Space'));
    const modes = h('div', { class: 'workspace-modes', role: 'group', 'aria-label': 'Desktop mode' }, ...(['gaming', 'productivity'] as const).map(m => h('button', {
      class: 'btn', 'aria-pressed': String(current === m), onclick: () => {
        if (m !== mode()) store.updateLayout(l => { l.desktop.workspace = { ...l.desktop.workspace, mode: m, notes: l.desktop.workspace?.notes ?? '' }; });
      },
    }, icon(m === 'gaming' ? 'gamepad' : 'grid', 14), m === 'gaming' ? 'Gaming' : 'Productivity')));
    header.insertBefore(modes, header.querySelector('.header-actions'));
    showHome();
    return () => { alive = false; bridge.send('desktop.panel', { active: false }); systemDispose?.(); offs.forEach(off => off()); area.remove(); };
  }
  const offOpen = bridge.on<Page>('desktop.open', p => { if (isMainOutput(output)) { store.setEditMode(false); openPage?.(p); } });
  const localOpen = (e: Event) => { if (isMainOutput(output)) openPage?.((e as CustomEvent<Page>).detail); };
  const appTitle = (e: Event) => {
    const el = root.querySelector('.gaming-edition');
    if (el && root.querySelector('.system-page-open')) el.textContent = `// ${(e as CustomEvent<string>).detail.replace(' · ', ' / ')}`;
  };
  window.addEventListener('desktop.open', localOpen); window.addEventListener('desktop.title', appTitle);
  mount();
  const offs = [store.on('layout', () => mount(true)), store.on('outputs', () => mount()), store.on('prefs', () => mount()), offOpen];
  return () => { dispose?.(); offs.forEach(off => off()); window.removeEventListener('desktop.open', localOpen); window.removeEventListener('desktop.title', appTitle); };
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
  rail.append(card('Notes', notes));
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
  return () => { alive = false; flushNotes(); disposeFiles(); offApps(); offLayout(); };
}
