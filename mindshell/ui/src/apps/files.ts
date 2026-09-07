// The Files app: a small file manager over the host's fs.* methods. Places on
// the left, a grid or list of the folder on the right, an inline context menu
// (app windows have no popups) and the usual keys.

import * as bridge from '../bridge';
import { clamp, h } from '../dom';
import { icon } from '../icons';
import { store } from '../state';
import type { FsEntry, FsListing, FsStat, Place } from '../types';
import { dialog, fmtBytes, fmtDate, frostSidebar, notice, row, setTitle, thumbUrl } from './shared';

interface Clip {
  paths: string[];
  cut: boolean;
}

interface MenuEntry {
  label?: string;
  icon?: string;
  danger?: boolean;
  disabled?: boolean;
  separator?: boolean;
  run?: () => void;
}

type SortKey = 'name' | 'size' | 'mtime';

const typeLabel = (e: FsEntry): string => {
  if (e.dir) return e.symlink ? 'Link to folder' : 'Folder';
  const [kind, sub = ''] = e.mime.split('/');
  const ext = e.name.includes('.') ? e.name.split('.').pop()!.toUpperCase() : '';
  if (kind === 'image') return `${ext || sub.toUpperCase()} image`;
  if (kind === 'video') return `${ext || sub.toUpperCase()} video`;
  if (kind === 'audio') return `${ext || sub.toUpperCase()} audio`;
  if (kind === 'text') return sub === 'plain' ? 'Text' : `${sub.replace(/^x-/, '')} text`;
  if (sub === 'x-executable' || sub === 'x-sharedlib') return 'Program';
  if (sub === 'x-desktop') return 'Launcher';
  if (sub.includes('zip') || sub.includes('tar') || sub.includes('compress') || sub.includes('7z') || sub.includes('rar')) return 'Archive';
  if (sub === 'pdf') return 'PDF document';
  return sub ? sub.replace(/^(x-|vnd\.)/, '').replace(/[-.]/g, ' ') : 'File';
};

const baseName = (p: string) => p.split('/').filter(Boolean).pop() ?? '/';

export function renderFiles(root: HTMLElement, start?: string): void {
  root.classList.add('app-files');
  const note = notice();
  let cwd = '';
  let listing: FsListing | undefined;
  let shown: FsEntry[] = [];
  let history: string[] = [];
  let hist = -1;
  let selected = new Set<string>();
  let anchor: string | undefined;
  let view: 'grid' | 'list' = 'grid';
  let hidden = false;
  let clip: Clip | undefined;
  let filter = '';
  let places: Place[] = [];
  let sortKey: SortKey = 'name';
  let sortDesc = false;
  let typeahead = '';
  let typeTimer: ReturnType<typeof setTimeout> | undefined;

  try {
    view = localStorage.getItem('files.view') === 'list' ? 'list' : 'grid';
    hidden = localStorage.getItem('files.hidden') === '1';
  } catch {
    /* no storage */
  }
  const urlView = new URLSearchParams(window.location.search).get('view');
  if (urlView === 'list' || urlView === 'grid') view = urlView;
  const remember = () => {
    try {
      localStorage.setItem('files.view', view);
      localStorage.setItem('files.hidden', hidden ? '1' : '0');
    } catch {
      /* no storage */
    }
  };

  const fail = (e: unknown) => note.show(e instanceof Error ? e.message : String(e), 'error');

  // ----- chrome ---------------------------------------------------------------

  const tool = (ic: string, title: string, onclick: () => void, cls = '') => h('button', { class: `tool ${cls}`.trim(), title, onclick }, icon(ic, 16));
  const backBtn = tool('arrow-left', 'Back (Alt+←)', () => go(-1));
  const fwdBtn = tool('arrow-right', 'Forward (Alt+→)', () => go(1));
  const upBtn = tool('arrow-up', 'Up (Backspace)', () => listing?.parent && navigate(listing.parent));
  const crumbs = h('div', { class: 'crumbs', title: 'Click the empty space to type a path (Ctrl+L)' });
  const location = h('input', { type: 'text', class: 'location mono', hidden: true, spellcheck: 'false' }) as HTMLInputElement;
  const crumbWrap = h('div', { class: 'crumb-wrap' }, crumbs, location);
  const search = h('input', { type: 'search', class: 'filter', placeholder: 'Filter', spellcheck: 'false' }) as HTMLInputElement;
  const gridBtn = h('button', { class: 'seg', title: 'Icons', onclick: () => setView('grid') }, icon('grid-view', 14));
  const listBtn = h('button', { class: 'seg', title: 'List', onclick: () => setView('list') }, icon('list', 14));
  const hiddenBtn = tool('eye', 'Show hidden files (Ctrl+H)', () => toggleHidden());
  const newBtn = h('button', { class: 'btn small', title: 'New folder (Ctrl+N)', onclick: () => newFolder() }, icon('plus', 14), 'Folder');
  const top = h('div', { class: 'files-top' }, backBtn, fwdBtn, upBtn, crumbWrap, search, h('span', { class: 'segs' }, gridBtn, listBtn), hiddenBtn, newBtn);
  const side = h('aside', { class: 'files-side' });
  const body = h('div', { class: 'files-body', tabindex: 0 });
  const status = h('div', { class: 'files-status' });
  const menu = h('div', { class: 'inline-menu menu', hidden: true });
  root.append(h('div', { class: 'files-head' }, top, note.el), h('div', { class: 'files-main' }, side, body), status, menu);
  frostSidebar(side);

  const setView = (v: 'grid' | 'list') => {
    view = v;
    remember();
    render();
  };
  const toggleHidden = () => {
    hidden = !hidden;
    remember();
    void load(cwd);
  };

  // ----- navigation -----------------------------------------------------------

  const load = async (path: string, keep: string[] = []) => {
    const l = await bridge.call<FsListing>('fs.list', { path, hidden });
    listing = l;
    cwd = l.path;
    selected = new Set(keep.filter((p) => l.entries.some((e) => e.path === p)));
    anchor = keep[0];
    setTitle(`${baseName(cwd)} · Files`);
    render();
  };
  const navigate = (path: string) =>
    load(path)
      .then(() => {
        history = history.slice(0, hist + 1);
        history.push(cwd);
        hist = history.length - 1;
        syncNav();
        body.focus();
      })
      .catch(fail);
  const go = (d: number) => {
    const i = hist + d;
    if (i < 0 || i >= history.length) return;
    hist = i;
    load(history[i]).then(syncNav).catch(fail);
  };
  const reload = (keep: string[] = [...selected]) => load(cwd, keep).catch(fail);
  const syncNav = () => {
    backBtn.disabled = hist <= 0;
    fwdBtn.disabled = hist >= history.length - 1;
    upBtn.disabled = !listing?.parent;
  };

  const showLocation = () => {
    location.value = cwd;
    location.hidden = false;
    crumbs.hidden = true;
    location.focus();
    location.select();
  };
  const hideLocation = () => {
    location.hidden = true;
    crumbs.hidden = false;
  };
  crumbs.addEventListener('click', (e) => {
    if (e.target === crumbs) showLocation();
  });
  location.addEventListener('keydown', (e) => {
    if (e.key === 'Enter') {
      hideLocation();
      void navigate(location.value.trim() || '/');
    } else if (e.key === 'Escape') hideLocation();
  });
  location.addEventListener('blur', hideLocation);

  // ----- selection + opening --------------------------------------------------

  const open = (e: FsEntry) => {
    if (e.dir) void navigate(e.path);
    else bridge.send('fs.open', { path: e.path });
  };
  const selectOnly = (path: string) => {
    selected = new Set([path]);
    anchor = path;
    syncSelection();
  };
  const clickSelect = (e: FsEntry, ev: MouseEvent) => {
    if (ev.ctrlKey || ev.metaKey) {
      if (selected.has(e.path)) selected.delete(e.path);
      else selected.add(e.path);
      anchor = e.path;
    } else if (ev.shiftKey && anchor) {
      const a = shown.findIndex((x) => x.path === anchor);
      const b = shown.findIndex((x) => x.path === e.path);
      if (a >= 0 && b >= 0) selected = new Set(shown.slice(Math.min(a, b), Math.max(a, b) + 1).map((x) => x.path));
    } else selectOnly(e.path);
    syncSelection();
  };
  const selectedEntries = () => shown.filter((e) => selected.has(e.path));

  // ----- operations -----------------------------------------------------------

  const copy = (cut: boolean) => {
    if (!selected.size) return;
    clip = { paths: [...selected], cut };
    renderStatus();
  };
  const paste = () => {
    if (!clip || !cwd) return;
    const c = clip;
    bridge
      .call<{ count: number }>(c.cut ? 'fs.move' : 'fs.copy', { paths: c.paths, dest: cwd })
      .then((r) => {
        if (c.cut) clip = undefined;
        note.show(`${c.cut ? 'Moved' : 'Copied'} ${r.count} item${r.count === 1 ? '' : 's'}.`, 'ok');
        void reload(c.paths.map((p) => `${cwd}/${baseName(p)}`));
      })
      .catch(fail);
  };
  const trash = () => {
    const paths = [...selected];
    if (!paths.length) return;
    bridge
      .call<{ count: number }>('fs.trash', { paths })
      .then((r) => {
        note.show(`Moved ${r.count} item${r.count === 1 ? '' : 's'} to the trash.`, 'ok');
        void reload([]);
      })
      .catch(fail);
  };
  const newFolder = () => {
    bridge
      .call<{ path: string }>('fs.mkdir', { path: cwd, name: 'New folder' })
      .then((r) => load(cwd, [r.path]).then(() => rename(r.path)))
      .catch(fail);
  };
  const rename = (path: string) => {
    const item = body.querySelector(`[data-path="${CSS.escape(path)}"]`) as HTMLElement | null;
    const nameEl = item?.querySelector('.fi-name, .fr-name') as HTMLElement | null;
    if (!item || !nameEl) return;
    const e = shown.find((x) => x.path === path);
    if (!e) return;
    const input = h('input', { type: 'text', class: 'rename', value: e.name, spellcheck: 'false' }) as HTMLInputElement;
    nameEl.replaceChildren(input);
    item.classList.add('renaming');
    const stop = (commit: boolean) => {
      const name = input.value.trim();
      input.remove();
      nameEl.textContent = e.name;
      item.classList.remove('renaming');
      body.focus();
      if (!commit || !name || name === e.name) return;
      bridge
        .call<{ path: string }>('fs.rename', { path, name })
        .then((r) => reload([r.path]))
        .catch(fail);
    };
    input.addEventListener('keydown', (ev) => {
      ev.stopPropagation();
      if (ev.key === 'Enter') stop(true);
      else if (ev.key === 'Escape') stop(false);
    });
    input.addEventListener('blur', () => stop(true));
    input.addEventListener('click', (ev) => ev.stopPropagation());
    input.addEventListener('dblclick', (ev) => ev.stopPropagation());
    input.focus();
    const dot = e.dir ? -1 : e.name.lastIndexOf('.');
    input.setSelectionRange(0, dot > 0 ? dot : e.name.length);
  };
  const properties = (path: string) => {
    bridge
      .call<FsStat>('fs.stat', { path })
      .then((s) => {
        const body = h(
          'div',
          { class: 'props' },
          row('Name', null, h('span', { class: 'mono' }, s.name)),
          row('Where', null, h('span', { class: 'mono' }, s.path.slice(0, -(s.name.length + 1)) || '/')),
          row('Type', null, h('span', {}, s.dir ? 'Folder' : s.mime)),
          s.dir ? row('Contains', null, h('span', {}, s.items === undefined ? '…' : `${s.items} item${s.items === 1 ? '' : 's'}`)) : row('Size', null, h('span', { class: 'mono' }, `${fmtBytes(s.size)} (${s.size.toLocaleString()} bytes)`)),
          row('Modified', null, h('span', {}, fmtDate(s.mtime))),
          s.link ? row('Points to', null, h('span', { class: 'mono' }, s.link)) : null,
          row('Permissions', null, h('span', { class: 'mono' }, `${s.permissions} (${s.mode.toString(8)})`)),
        );
        const close = dialog(root, 'Properties', body, [h('button', { class: 'btn primary', onclick: () => close() }, 'Close')]);
      })
      .catch(fail);
  };
  const setWallpaper = (path: string) => {
    void store.updateLayout((l) => (l.desktop.wallpaper = { mode: 'image', path }));
    note.show(`${baseName(path)} is now the wallpaper.`, 'ok');
  };
  const openTerminal = () => bridge.send('shell.exec', { cmd: `${store.state.config.terminal || 'foot'} -D ${JSON.stringify(cwd)}` });

  // ----- inline menu ----------------------------------------------------------

  const hideMenu = () => (menu.hidden = true);
  const showMenu = (x: number, y: number, items: MenuEntry[]) => {
    menu.replaceChildren();
    for (const it of items) {
      if (it.separator) {
        menu.appendChild(h('div', { class: 'menu-sep' }));
        continue;
      }
      const b = h('button', { class: `menu-item${it.danger ? ' danger' : ''}`, disabled: !!it.disabled }, h('span', { class: 'menu-ic' }, it.icon ? icon(it.icon, 14) : null), h('span', { class: 'menu-label' }, it.label ?? ''));
      b.addEventListener('click', () => {
        hideMenu();
        it.run?.();
      });
      menu.appendChild(b);
    }
    menu.hidden = false;
    const r = root.getBoundingClientRect();
    const scale = r.width / root.offsetWidth || 1;
    const px = (x - r.left) / scale;
    const py = (y - r.top) / scale;
    const mw = menu.offsetWidth;
    const mh = menu.offsetHeight;
    menu.style.left = `${clamp(px, 4, root.offsetWidth - mw - 4)}px`;
    menu.style.top = `${clamp(py, 4, root.offsetHeight - mh - 4)}px`;
  };
  const entryMenu = (e: FsEntry, ev: MouseEvent) => {
    if (!selected.has(e.path)) selectOnly(e.path);
    const many = selected.size > 1;
    const items: MenuEntry[] = [
      { label: e.dir ? 'Open' : 'Open with the default app', icon: e.dir ? 'folder-open' : 'external', run: () => open(e) },
      ...(e.image && !many ? [{ label: 'Set as wallpaper', icon: 'image', run: () => setWallpaper(e.path) }] : []),
      { separator: true },
      { label: 'Cut', icon: 'cut', run: () => copy(true) },
      { label: 'Copy', icon: 'copy', run: () => copy(false) },
      { label: 'Paste', icon: 'paste', disabled: !clip, run: paste },
      { separator: true },
      { label: 'Rename', icon: 'rename', disabled: many, run: () => rename(e.path) },
      { label: many ? `Move ${selected.size} items to the trash` : 'Move to the trash', icon: 'trash', danger: true, run: trash },
      { separator: true },
      { label: 'Properties', icon: 'info', disabled: many, run: () => properties(e.path) },
    ];
    showMenu(ev.clientX, ev.clientY, items);
  };
  const folderMenu = (ev: MouseEvent) => {
    selected.clear();
    syncSelection();
    showMenu(ev.clientX, ev.clientY, [
      { label: 'New folder', icon: 'plus', run: newFolder },
      { label: 'Paste', icon: 'paste', disabled: !clip, run: paste },
      { separator: true },
      { label: 'Open a terminal here', icon: 'terminal', run: openTerminal },
      { label: hidden ? 'Hide hidden files' : 'Show hidden files', icon: 'eye', run: toggleHidden },
      { label: 'Select all', icon: 'check', run: selectAll },
      { separator: true },
      { label: 'Properties', icon: 'info', run: () => properties(cwd) },
    ]);
  };
  const selectAll = () => {
    selected = new Set(shown.map((e) => e.path));
    syncSelection();
  };
  root.addEventListener('pointerdown', (e) => {
    if (!menu.hidden && !menu.contains(e.target as Node)) hideMenu();
  });

  // ----- rendering ------------------------------------------------------------

  const iconFor = (e: FsEntry, size: number) => (e.thumb ? e.thumb : e.image ? thumbUrl(e.path, size) : e.icon);

  const renderCrumbs = () => {
    crumbs.replaceChildren();
    const parts = cwd.split('/').filter(Boolean);
    const home = places.find((p) => p.kind === 'home')?.path;
    let acc = '';
    const seg = (label: string, path: string, ic?: string) => {
      const b = h('button', { class: 'crumb', onclick: () => void navigate(path) }, ic ? icon(ic, 13) : null, h('span', {}, label));
      crumbs.appendChild(b);
    };
    if (home && (cwd === home || cwd.startsWith(home + '/'))) {
      seg('Home', home, 'home');
      acc = home;
      const rest = cwd.slice(home.length).split('/').filter(Boolean);
      for (const p of rest) {
        acc += '/' + p;
        seg(p, acc);
      }
    } else {
      seg('System', '/', 'hdd');
      for (const p of parts) {
        acc += '/' + p;
        seg(p, acc);
      }
    }
    crumbs.lastElementChild?.classList.add('on');
  };

  const renderPlaces = () => {
    // Keep the frosted backdrop (frostSidebar), drop the rest.
    for (const el of Array.from(side.children)) if (!el.classList.contains('glass-bd')) el.remove();
    const group = (title: string, list: Place[]) => {
      if (!list.length) return;
      side.appendChild(h('div', { class: 'side-title' }, title));
      for (const p of list) {
        const b = h('button', { class: 'place', dataset: { path: p.path } }, h('span', { class: 'nav-ic' }, icon(p.kind === 'home' ? 'home' : p.kind === 'system' ? 'hdd' : p.kind === 'mount' ? (p.removable ? 'usb' : 'hdd') : p.icon || 'folder', 16)), h('span', { class: 'place-name' }, p.name));
        b.addEventListener('click', () => void navigate(p.path));
        side.appendChild(b);
      }
    };
    group('Places', places.filter((p) => p.kind === 'home' || p.kind === 'folder'));
    group('Devices', places.filter((p) => p.kind === 'system' || p.kind === 'mount'));
    syncPlaces();
  };
  const syncPlaces = () => {
    let best = '';
    for (const p of places) if ((cwd === p.path || cwd.startsWith(p.path.replace(/\/?$/, '/'))) && p.path.length > best.length) best = p.path;
    side.querySelectorAll('.place').forEach((b) => b.classList.toggle('on', (b as HTMLElement).dataset.path === best));
  };

  const sorted = (): FsEntry[] => {
    const q = filter.trim().toLowerCase();
    const list = (listing?.entries ?? []).filter((e) => (hidden || !e.hidden) && (!q || e.name.toLowerCase().includes(q)));
    const dir = sortDesc ? -1 : 1;
    list.sort((a, b) => {
      if (a.dir !== b.dir) return a.dir ? -1 : 1;
      if (sortKey === 'size') return (a.size - b.size) * dir || a.name.localeCompare(b.name);
      if (sortKey === 'mtime') return (a.mtime - b.mtime) * dir || a.name.localeCompare(b.name);
      return a.name.localeCompare(b.name, undefined, { numeric: true, sensitivity: 'base' }) * dir;
    });
    return list;
  };

  const renderBody = () => {
    shown = sorted();
    body.replaceChildren();
    body.dataset.view = view;
    if (view === 'grid') {
      const grid = h('div', { class: 'fgrid' });
      for (const e of shown) {
        const img = h('img', { class: 'fi-ic', src: iconFor(e, 128), alt: '', draggable: false, loading: 'lazy' });
        const it = h('div', { class: `fi${e.image ? ' img' : ''}${e.hidden ? ' hid' : ''}`, dataset: { path: e.path }, title: `${e.name}\n${typeLabel(e)}${e.dir ? '' : ' · ' + fmtBytes(e.size)}` }, h('span', { class: 'fi-frame' }, img), h('span', { class: 'fi-name' }, e.name));
        wire(it, e);
        grid.appendChild(it);
      }
      body.appendChild(grid);
    } else {
      const head = h('div', { class: 'fr fr-head' }, ...(['name', 'size', 'mtime', 'type'] as const).map((k) => {
        const b = h('button', { class: `fr-${k === 'mtime' ? 'date' : k} fr-h${sortKey === k ? (sortDesc ? ' desc' : ' asc') : ''}`, onclick: () => {
          if (k === 'type') return;
          if (sortKey === k) sortDesc = !sortDesc;
          else {
            sortKey = k;
            sortDesc = k !== 'name';
          }
          render();
        } }, k === 'name' ? 'Name' : k === 'size' ? 'Size' : k === 'mtime' ? 'Modified' : 'Type');
        return b;
      }));
      const list = h('div', { class: 'flist' }, head);
      for (const e of shown) {
        const it = h(
          'div',
          { class: `fr${e.hidden ? ' hid' : ''}`, dataset: { path: e.path } },
          h('span', { class: 'fr-name' }, h('img', { class: 'fr-ic', src: iconFor(e, 32), alt: '', draggable: false }), h('span', { class: 'fr-text' }, e.name)),
          h('span', { class: 'fr-size mono' }, e.dir ? '' : fmtBytes(e.size)),
          h('span', { class: 'fr-date' }, fmtDate(e.mtime)),
          h('span', { class: 'fr-type' }, typeLabel(e)),
        );
        wire(it, e);
        list.appendChild(it);
      }
      body.appendChild(list);
    }
    if (!shown.length) body.appendChild(h('div', { class: 'fempty' }, filter ? 'Nothing matches the filter.' : hidden ? 'This folder is empty.' : 'This folder is empty (hidden files are not shown).'));
    syncSelection();
  };
  const wire = (it: HTMLElement, e: FsEntry) => {
    it.addEventListener('click', (ev) => clickSelect(e, ev));
    it.addEventListener('dblclick', (ev) => {
      ev.preventDefault();
      open(e);
    });
    it.addEventListener('contextmenu', (ev) => {
      ev.preventDefault();
      ev.stopPropagation();
      entryMenu(e, ev);
    });
  };
  body.addEventListener('click', (e) => {
    if (e.target === body || (e.target as HTMLElement).classList.contains('fgrid') || (e.target as HTMLElement).classList.contains('flist')) {
      selected.clear();
      syncSelection();
    }
  });
  body.addEventListener('contextmenu', (e) => {
    e.preventDefault();
    folderMenu(e);
  });

  const syncSelection = () => {
    body.querySelectorAll('[data-path]').forEach((el) => el.classList.toggle('sel', selected.has((el as HTMLElement).dataset.path ?? '')));
    renderStatus();
  };
  const renderStatus = () => {
    const sel = selectedEntries();
    const bytes = sel.reduce((n, e) => n + (e.dir ? 0 : e.size), 0);
    const parts = [`${shown.length} item${shown.length === 1 ? '' : 's'}`];
    if (sel.length) parts.push(`${sel.length} selected${bytes ? ` · ${fmtBytes(bytes)}` : ''}`);
    if (clip) parts.push(`${clip.paths.length} ${clip.cut ? 'cut' : 'copied'} · Ctrl+V to paste`);
    status.replaceChildren(...parts.map((t) => h('span', {}, t)));
  };
  const render = () => {
    renderCrumbs();
    renderBody();
    syncPlaces();
    gridBtn.classList.toggle('on', view === 'grid');
    listBtn.classList.toggle('on', view === 'list');
    hiddenBtn.classList.toggle('on', hidden);
  };

  // ----- keys -----------------------------------------------------------------

  const columns = () => {
    const grid = body.querySelector('.fgrid');
    if (!grid) return 1;
    const cols = getComputedStyle(grid).gridTemplateColumns.split(' ').filter(Boolean).length;
    return Math.max(1, cols);
  };
  const moveSel = (d: number, extend: boolean) => {
    if (!shown.length) return;
    const i = anchor ? shown.findIndex((e) => e.path === anchor) : -1;
    const j = clamp(i < 0 ? 0 : i + d, 0, shown.length - 1);
    const e = shown[j];
    if (extend) selected.add(e.path);
    else selected = new Set([e.path]);
    anchor = e.path;
    syncSelection();
    (body.querySelector(`[data-path="${CSS.escape(e.path)}"]`) as HTMLElement | null)?.scrollIntoView({ block: 'nearest' });
  };
  search.addEventListener('input', () => {
    filter = search.value;
    renderBody();
  });
  search.addEventListener('keydown', (e) => {
    if (e.key === 'Escape') {
      search.value = '';
      filter = '';
      renderBody();
      body.focus();
    } else if (e.key === 'Enter') {
      if (shown.length) selectOnly(shown[0].path);
      body.focus();
    }
    e.stopPropagation();
  });
  root.addEventListener('keydown', (e) => {
    const t = e.target as HTMLElement;
    if (t instanceof HTMLInputElement || t instanceof HTMLTextAreaElement) return;
    const ctrl = e.ctrlKey || e.metaKey;
    const k = e.key;
    const handled = () => e.preventDefault();
    if (ctrl && k.toLowerCase() === 'a') return handled(), selectAll();
    if (ctrl && k.toLowerCase() === 'c') return handled(), copy(false);
    if (ctrl && k.toLowerCase() === 'x') return handled(), copy(true);
    if (ctrl && k.toLowerCase() === 'v') return handled(), paste();
    if (ctrl && k.toLowerCase() === 'h') return handled(), toggleHidden();
    if (ctrl && k.toLowerCase() === 'n') return handled(), newFolder();
    if (ctrl && k.toLowerCase() === 'l') return handled(), showLocation();
    if (ctrl && k.toLowerCase() === 'f') return handled(), search.focus();
    if (e.altKey && k === 'ArrowLeft') return handled(), go(-1);
    if (e.altKey && k === 'ArrowRight') return handled(), go(1);
    if ((e.altKey && k === 'ArrowUp') || (k === 'Backspace' && !ctrl)) return handled(), void (listing?.parent && navigate(listing.parent));
    if (k === 'Delete') return handled(), trash();
    if (k === 'F2' && selected.size === 1) return handled(), rename([...selected][0]);
    if (k === 'F5') return handled(), void reload();
    if (k === 'Enter') {
      const sel = selectedEntries();
      if (sel.length === 1) return handled(), open(sel[0]);
      for (const s of sel) if (!s.dir) bridge.send('fs.open', { path: s.path });
      return handled();
    }
    if (k === 'Escape') {
      handled();
      if (!menu.hidden) return hideMenu();
      if (filter) {
        search.value = '';
        filter = '';
        return renderBody();
      }
      selected.clear();
      return syncSelection();
    }
    if (k === 'ArrowLeft' || k === 'ArrowRight' || k === 'ArrowUp' || k === 'ArrowDown') {
      handled();
      const cols = view === 'grid' ? columns() : 1;
      const d = k === 'ArrowLeft' ? -1 : k === 'ArrowRight' ? 1 : k === 'ArrowUp' ? -cols : cols;
      return moveSel(d, e.shiftKey);
    }
    if (k === 'Home' || k === 'End') {
      handled();
      if (shown.length) selectOnly(shown[k === 'Home' ? 0 : shown.length - 1].path);
      return;
    }
    if (k.length === 1 && !ctrl && !e.altKey) {
      // Type-ahead: jump to the first name starting with what was typed.
      handled();
      typeahead += k.toLowerCase();
      if (typeTimer) clearTimeout(typeTimer);
      typeTimer = setTimeout(() => (typeahead = ''), 900);
      const hit = shown.find((x) => x.name.toLowerCase().startsWith(typeahead));
      if (hit) {
        selectOnly(hit.path);
        (body.querySelector(`[data-path="${CSS.escape(hit.path)}"]`) as HTMLElement | null)?.scrollIntoView({ block: 'nearest' });
      }
    }
  });

  // ----- start ----------------------------------------------------------------

  render();
  bridge
    .call<Place[]>('fs.places')
    .then((p) => {
      places = Array.isArray(p) ? p : [];
      renderPlaces();
      renderCrumbs();
    })
    .catch(fail);
  const first = start && start.trim() ? start.trim() : undefined;
  (first ? Promise.resolve({ path: first }) : bridge.call<{ path: string }>('fs.home'))
    .then((r) => navigate(r.path))
    .catch((e) => {
      fail(e);
      void navigate('/');
    });
}
