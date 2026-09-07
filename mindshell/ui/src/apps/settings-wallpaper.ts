// Settings › Wallpaper: a grid of the pictures found on the system.

import * as bridge from '../bridge';
import { h } from '../dom';
import { icon } from '../icons';
import { store } from '../state';
import type { FsListing, WallpaperEntry } from '../types';
import { card, notice, pageHeader, thumbUrl } from './shared';

export function wallpaperPage(el: HTMLElement): () => void {
  const note = notice();
  const grid = h('div', { class: 'wp-grid' });
  const extra: WallpaperEntry[] = [];
  let entries: WallpaperEntry[] = [];

  const folderIn = h('input', { type: 'text', class: 'grow', placeholder: '~/Pictures/Wallpapers', spellcheck: 'false' }) as HTMLInputElement;
  const addBtn = h('button', { class: 'btn', onclick: () => addFolder(folderIn.value.trim()) }, icon('folder', 14), 'Add pictures from this folder');
  folderIn.addEventListener('keydown', (e) => {
    if (e.key === 'Enter') addFolder(folderIn.value.trim());
  });

  el.append(
    pageHeader('Wallpaper', 'Pick a picture for the desktop. Anything in ~/Pictures/Wallpapers or /usr/share/backgrounds shows up here.'),
    note.el,
    card(null, grid),
    card('More pictures', h('div', { class: 'inline-form' }, folderIn, addBtn), h('div', { class: 'row-help' }, 'Or right-click a picture in Files or Image Viewer and choose “Set as Background”.')),
  );

  const current = () => store.state.layout.desktop.wallpaper;
  const choose = (path: string | null) => {
    void store.updateLayout((l) => {
      l.desktop.wallpaper = path ? { mode: 'image', path } : { mode: 'builtin' };
    });
  };

  const tile = (name: string, sub: string, preview: HTMLElement, selected: boolean, onPick: () => void) => {
    const t = h('button', { class: `wp-tile${selected ? ' on' : ''}`, title: sub ? `${name}\n${sub}` : name }, h('span', { class: 'wp-thumb' }, preview, h('span', { class: 'wp-check' }, icon('check', 14))), h('span', { class: 'wp-name' }, name));
    t.addEventListener('click', onPick);
    return t;
  };

  const render = () => {
    const wp = current();
    grid.replaceChildren();
    grid.appendChild(tile('MindOS', 'The built-in grid', h('span', { class: 'wallpaper builtin' }), wp.mode !== 'image' || !wp.path, () => choose(null)));
    const all = [...entries, ...extra.filter((x) => !entries.some((e) => e.path === x.path))];
    const folders = new Map<string, WallpaperEntry[]>();
    for (const e of all) {
      const list = folders.get(e.folder) ?? [];
      list.push(e);
      folders.set(e.folder, list);
    }
    for (const [folder, list] of folders) {
      grid.appendChild(h('h3', { class: 'wp-folder' }, folder));
      for (const e of list) {
        const img = h('img', { src: e.thumb ?? thumbUrl(e.path, 320), alt: '', loading: 'lazy', draggable: false });
        grid.appendChild(tile(e.name, e.path, img, wp.mode === 'image' && wp.path === e.path, () => choose(e.path)));
      }
    }
    if (!all.length) grid.appendChild(h('div', { class: 'row-help' }, 'No pictures found yet. Put some in ~/Pictures/Wallpapers, or add a folder below.'));
  };

  const addFolder = (path: string) => {
    if (!path) return;
    bridge
      .call<FsListing>('fs.list', { path, hidden: false })
      .then((l) => {
        const images = l.entries.filter((e) => e.image && !e.dir);
        if (!images.length) return note.show('No pictures in that folder.', 'info');
        const folder = l.path.split('/').filter(Boolean).pop() ?? l.path;
        for (const e of images) if (!extra.some((x) => x.path === e.path)) extra.push({ path: e.path, name: e.name.replace(/\.[^.]+$/, ''), folder });
        note.show(`Added ${images.length} picture${images.length === 1 ? '' : 's'} from ${l.path}.`, 'ok');
        render();
      })
      .catch((e) => note.show(String(e instanceof Error ? e.message : e), 'error'));
  };

  render();
  bridge
    .call<WallpaperEntry[]>('wallpaper.list')
    .then((list) => {
      entries = Array.isArray(list) ? list : [];
      render();
    })
    .catch((e) => note.show(`Could not list wallpapers: ${e instanceof Error ? e.message : e}`, 'error'));
  store.bind(grid, 'layout', render);
  return () => undefined;
}
