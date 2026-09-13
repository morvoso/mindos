// Desktop icons: the Desktop folder laid out as a grid on the wallpaper,
// Windows-style (column by column from the top left). One click or two opens
// an item (Settings › Desktop chooses), Ctrl-click picks several out, right
// click gets a menu; the host says when the folder changes
// (`desktop.changed`) and the grid re-lists.

import * as actions from './actions';
import { launchWithFeedback } from './app-match';
import { thumbUrl } from './apps/shared';
import * as bridge from './bridge';
import { h, reconcile } from './dom';
import { desktopBar, onDesktopBar, panelsOn } from './geometry';
import { store } from './state';
import type { AppInfo, FsEntry, FsListing, MenuAction } from './types';

const PAD = 12;

export function renderDesktopIcons(root: HTMLElement, grid: HTMLElement, output: string): () => void {
  let dir = '';
  let entries: FsEntry[] = [];
  let gen = 0;
  const selected = new Set<string>();

  // A shortcut on the desktop is drawn as the thing it points at: the host
  // reads the `.desktop` or `.lnk` and sends back the application's own name
  // and icon. An entry that is also in the app index launches through the
  // index, which knows when its window appears.
  const appFor = (e: FsEntry): AppInfo | undefined => (e.shortcut && !e.dir ? store.state.apps.find((a) => a.name === e.label) : undefined);
  const iconFor = (e: FsEntry): string => (e.thumb ? e.thumb : e.image ? thumbUrl(e.path, 128) : e.icon);
  const labelFor = (e: FsEntry): string => e.label || (e.dir ? e.name : e.name.replace(/\.(desktop|lnk)$/i, ''));

  /** Single or double click to open, chosen in Settings › Desktop. */
  const activation = () => store.state.layout.desktop.workspace?.activate ?? 'single';

  // `fs.open` starts a shortcut's program; it is only the mime database that
  // would have handed a `.desktop` file to a text editor.
  const openAction = (e: FsEntry) => {
    const app = appFor(e);
    if (app) return { call: 'apps.launch', params: { id: app.id } };
    return { call: 'fs.open', params: { path: e.path } };
  };

  // Wine writes both files for one program: the installer's own `X.lnk` and
  // the `X.desktop` its menu builder made from it. One program, one icon.
  const shadowed = (list: FsEntry[]): FsEntry[] => {
    const desktops = new Set(list.filter((e) => !e.dir && /\.desktop$/i.test(e.name)).map((e) => e.name.slice(0, -8).toLowerCase()));
    return list.filter((e) => !(/\.lnk$/i.test(e.name) && desktops.has(e.name.slice(0, -4).toLowerCase())));
  };

  const syncSelection = () => {
    for (const el of Array.from(grid.children) as HTMLElement[]) el.classList.toggle('sel', selected.has(el.dataset.key ?? ''));
  };

  const item = (e: FsEntry): HTMLElement => {
    const img = h('img', { class: 'di-ic', src: iconFor(e), alt: '', draggable: false });
    const el = h('button', { class: `di${e.image ? ' img' : ''}`, title: e.name }, h('span', { class: 'di-frame' }, img, h('span', { class: 'di-spin' })), h('span', { class: 'di-name' }, labelFor(e)));
    // Opening something takes a moment; say so on the icon itself. An app gets
    // the real "its window appeared" signal, anything else a short flash.
    const open = () => {
      const app = appFor(e);
      if (app) return void launchWithFeedback(el, app.id);
      el.classList.add('launching');
      setTimeout(() => el.classList.remove('launching'), 1600);
      void actions.runAction(openAction(e));
    };
    el.addEventListener('click', (ev) => {
      const multi = ev.ctrlKey || ev.metaKey;
      if (!multi) selected.clear();
      if (selected.has(e.path)) selected.delete(e.path);
      else selected.add(e.path);
      syncSelection();
      // Single-click activation: the click both selects and opens, unless the
      // user is picking several items out.
      if (!multi && activation() === 'single') open();
    });
    el.addEventListener('dblclick', () => {
      if (activation() !== 'single') open();
    });
    el.addEventListener('contextmenu', (ev) => {
      ev.preventDefault();
      ev.stopPropagation();
      if (!selected.has(e.path)) {
        selected.clear();
        selected.add(e.path);
        syncSelection();
      }
      const paths = entries.filter((x) => selected.has(x.path)).map((x) => x.path);
      const many = paths.length > 1;
      const items: MenuAction[] = [
        { label: many ? `Open ${paths.length} items` : e.dir ? 'Open folder' : appFor(e) ? 'Launch' : 'Open', icon: e.dir ? 'folder' : 'window', action: openAction(e), disabled: many },
        { label: 'Show in Files', icon: 'folder', action: { call: 'fs.open', params: { path: dir } } },
        { label: '', separator: true },
        ...(!many && e.shortcut && /\.desktop$/i.test(e.name)
          ? [{ label: 'Uninstall', icon: 'trash', action: { call: 'apps.uninstall', params: { path: e.path } } } as MenuAction]
          : []),
        { label: many ? `Move ${paths.length} items to trash` : 'Move to trash', icon: 'trash', danger: true, action: { call: 'fs.trash', params: { paths } } },
      ];
      const r = rect(ev);
      actions.openPopup('context-menu', { items, title: many ? undefined : labelFor(e), anchor: { x: r.x, y: r.y, w: 0, h: 0 } });
    });
    return el;
  };

  const rect = (ev: MouseEvent) => {
    const scale = root.getBoundingClientRect().width / root.offsetWidth || 1;
    const b = root.getBoundingClientRect();
    return { x: (ev.clientX - b.left) / scale, y: (ev.clientY - b.top) / scale };
  };

  const update = (el: HTMLElement, e: FsEntry) => {
    const img = el.querySelector('img') as HTMLImageElement | null;
    const src = iconFor(e);
    if (img && img.getAttribute('src') !== src) img.src = src;
    const name = el.querySelector('.di-name');
    const label = labelFor(e);
    if (name && name.textContent !== label) name.textContent = label;
    el.classList.toggle('img', !!e.image);
  };

  const render = () => {
    const show = store.state.layout.desktop.icons !== false;
    grid.hidden = !show;
    grid.classList.toggle('editing', store.state.editMode);
    // Keep clear of the panels (their exclusive zones), and of the desktop's
    // own bar, which is on screen whichever view the desktop is in. Its height
    // already counts any panel above it, so the larger of the two is the top.
    const pad = { top: PAD, right: PAD, bottom: PAD, left: PAD };
    for (const p of panelsOn(store.state.layout.panels, output)) {
      pad[p.edge] = Math.max(pad[p.edge], p.size + p.margin + PAD);
    }
    pad.top = Math.max(pad.top, desktopBar() + PAD);
    grid.style.padding = `${pad.top}px ${pad.right}px ${pad.bottom}px ${pad.left}px`;
    if (!show) return;
    for (const path of Array.from(selected)) if (!entries.some((e) => e.path === path)) selected.delete(path);
    reconcile(grid, entries, (e) => e.path, item, update, (el) => el.remove());
    syncSelection();
  };

  const load = async () => {
    const my = ++gen;
    try {
      if (!dir) dir = (await bridge.call<{ path: string }>('fs.desktop')).path;
      const listing = await bridge.call<FsListing>('fs.list', { path: dir });
      if (my !== gen) return;
      entries = shadowed(listing.entries.filter((e) => !e.hidden));
    } catch (e) {
      console.warn('desktop icons:', e);
      entries = [];
    }
    render();
  };

  grid.addEventListener('pointerdown', (ev) => {
    if (ev.target === grid && selected.size) {
      selected.clear();
      syncSelection();
    }
  });

  render();
  void load();
  const offs = [
    store.on('layout', render),
    store.on('editMode', render),
    store.on('outputs', render),
    store.on('apps', render),
    onDesktopBar(render),
    bridge.on('desktop.changed', () => void load()),
  ];
  return () => {
    gen++;
    offs.forEach((off) => off());
    grid.replaceChildren();
  };
}

/** The desktop's own right-click entries for the icons. */
export function desktopIconMenu(): MenuAction[] {
  const on = store.state.layout.desktop.icons !== false;
  return [
    { label: on ? 'Hide desktop icons' : 'Show desktop icons', icon: 'desktop', action: { desktopIcons: !on } },
    { label: 'Open Desktop folder', icon: 'folder', action: { call: 'fs.open', params: { path: '~/Desktop' } } },
  ];
}
