import * as bridge from './bridge';
import { appearanceControls } from './appearance';
import { renderSettings } from './apps/settings';
import { renderGaming } from './apps/gaming';
import { h } from './dom';
import { renderGamingDesktop } from './gaming-desktop';
import { icon } from './icons';
import { store } from './state';
import type { FsListing } from './types';

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
  function mount(animate = false) {
    const primary = isMainOutput(output);
    if (primary === previousPrimary && (!primary || mountedMode === mode())) return;
    dispose?.(); dispose = undefined; openPage = undefined;
    previousPrimary = primary; mountedMode = primary ? mode() : undefined;
    if (!primary) return;
    dispose = mountMode(mode(), animate);
  }
  function mountMode(current: Mode, animate: boolean): () => void {
    const offs: (() => void)[] = [];
    let systemDispose: (() => void) | undefined;
    let alive = true;
    let area: HTMLElement, nav: HTMLElement, main: HTMLElement, rail: HTMLElement, header: HTMLElement;
    if (current === 'gaming') {
      offs.push(renderGamingDesktop(root, output));
      area = root.querySelector('.gaming-workspace')!;
      nav = area.querySelector('.gaming-nav')!;
      main = area.querySelector('.gaming-main')!;
      rail = area.querySelector('.gaming-rail')!;
      header = area.querySelector('.gaming-menubar')!;
    } else {
      const appearance = appearanceControls(); offs.push(appearance.destroy);
      header = h('header', { class: 'gaming-menubar' }, h('strong', { class: 'gaming-brand' }, h('i'), 'MINDOS'), h('span', { class: 'gaming-meta gaming-edition' }, '// Home'), appearance.el);
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
      offs.push(renderProductivity(main, rail, output));
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
          void store.updateLayout(l => { l.desktop.workspace = { mode: 'gaming', notes: l.desktop.workspace?.notes ?? '' }; });
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
      ...(current === 'gaming' ? [link('Gaming Center', 'gamepad', open('gaming'), 'gaming'), link('Companion', 'monitor', run(() => bridge.call('shell.openApp', { name: 'companion' })))] : [
        link('Documents', 'folder', run(() => bridge.call('fs.open', { path: '~/Documents' }))),
        link('Office', 'edit', launch(/libreoffice.*startcenter|libreoffice-writer|onlyoffice/i)),
        link('Mail', 'mail', launch(/thunderbird|evolution|geary/i)),
      ]),
      link('Files', 'folder', run(() => bridge.call('fs.open', { path: '~' }))),
      link('Browser', 'globe', launch(/firefox|chromium/i)),
      link('Settings', 'gear', open('settings'), 'settings'),
      h('span', { class: 'gaming-nav-bottom gaming-meta' }, 'Super + Space', h('br'), 'Launch · Ask · Find'));
    const modes = h('div', { class: 'workspace-modes', role: 'group', 'aria-label': 'Desktop mode' }, ...(['gaming', 'productivity'] as const).map(m => h('button', {
      class: 'btn', 'aria-pressed': String(current === m), onclick: () => {
        if (m !== mode()) store.updateLayout(l => { l.desktop.workspace = { mode: m, notes: l.desktop.workspace?.notes ?? '' }; });
      },
    }, icon(m === 'gaming' ? 'gamepad' : 'grid', 14), m === 'gaming' ? 'Gaming' : 'Productivity')));
    header.insertBefore(modes, header.querySelector('.appearance-controls'));
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

function renderProductivity(main: HTMLElement, rail: HTMLElement, output: string): () => void {
  let alive = true;
  const status = h('p', { class: 'play-status', role: 'status' });
  const run = (fn: () => Promise<unknown>) => () => { void fn().catch(e => { if (alive) status.textContent = String(e); }); };
  const card = (title: string, ...body: HTMLElement[]) => h('section', { class: 'gaming-rail-card work-card' }, h('header', { class: 'gaming-panel-title' }, h('h2', {}, title)), ...body);
  const files = h('div', { class: 'work-files' }, h('p', { class: 'gaming-meta' }, 'Loading your documents…'));
  async function refresh() {
    try {
      const list = await bridge.call<FsListing>('fs.list', { path: '~/Documents', hidden: false });
      if (!alive) return;
      files.replaceChildren(...list.entries.filter(f => !f.hidden).sort((a, b) => b.mtime - a.mtime).slice(0, 6).map(f => h('button', { class: 'work-file', onclick: run(() => bridge.call('fs.open', { path: f.path })) }, icon(f.dir ? 'folder' : 'file', 18), h('span', {}, f.name), h('small', {}, new Date(f.mtime * 1000).toLocaleDateString()))));
      if (!files.children.length) files.append(h('p', { class: 'gaming-meta' }, 'Your Documents folder is empty.'));
    } catch { if (alive) files.replaceChildren(h('p', { class: 'gaming-meta' }, 'Create a Documents folder in Files to keep your work here.')); }
  }
  const launch = (pattern: RegExp) => run(async () => {
    const app = store.state.apps.find(a => pattern.test(a.id));
    if (app) await bridge.call('apps.launch', { id: app.id });
    else await bridge.call('shell.openApp', { name: 'settings', page: 'software' });
  });
  const shortcuts = h('div', { class: 'work-shortcuts' }, ...[
    ['Browser', 'globe', /firefox|chromium/i], ['Write', 'edit', /libreoffice-writer|onlyoffice/i], ['Spreadsheets', 'grid', /libreoffice-calc|onlyoffice/i], ['Mail', 'mail', /thunderbird|evolution|geary/i],
  ].map(([label, glyph, pattern]) => h('button', { class: 'work-shortcut', onclick: launch(pattern as RegExp) }, icon(String(glyph), 24), h('strong', {}, String(label)))));
  main.append(card('Home & work', h('div', { class: 'work-intro' }, shortcuts)),
    card('Recent documents', files, h('div', { class: 'play-row' }, h('button', { class: 'btn', onclick: run(() => bridge.call('fs.open', { path: '~/Documents' })) }, 'Open Documents'), h('button', { class: 'btn', onclick: () => void refresh() }, 'Refresh'))), status);
  const notes = h('textarea', { class: 'work-notes', placeholder: 'Type a note…', 'aria-label': 'Desktop notes', value: store.state.layout.desktop.workspace?.notes ?? '' });
  notes.value = store.state.layout.desktop.workspace?.notes ?? '';
  // Persist every edit. Switching modes cannot lose the last keystroke.
  notes.addEventListener('input', () => store.updateLayout(l => { l.desktop.workspace = { mode: l.desktop.workspace?.mode ?? 'productivity', notes: notes.value }; }));
  const windows = h('div', { class: 'work-windows' });
  const renderWindows = () => {
    const list = store.state.windows.filter(w => w.output === output || (!output && !w.output));
    windows.replaceChildren(...list.map(w => h('button', { class: 'work-file', onclick: run(() => bridge.call('windows.focus', { id: w.id })) }, icon('window', 16), h('span', {}, w.title || w.app_id))));
    if (!list.length) windows.append(h('p', { class: 'gaming-meta' }, 'No windows on this monitor.'));
  };
  rail.append(card('Notes', notes), card('This monitor', windows), card('Mind', h('div', { class: 'gaming-rail-body' }, h('button', { class: 'btn primary', onclick: run(() => bridge.call('mind.open', { text: 'Help me plan my day.' })) }, 'Ask Mind'))));
  renderWindows(); void refresh();
  const off = store.on('windows', renderWindows);
  return () => { alive = false; off(); };
}
