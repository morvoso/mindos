// Settings › Appearance: the colour scheme (presets, the seven colours behind
// every token, and palettes the user saved) and the desktop background.

import * as bridge from '../bridge';
import {
  appearance, appearanceControls, deleteSavedPalette, onAppearance, paletteFor, PRESETS, savedPalettes,
  savePalette, setPaletteColor, usePreset, useSavedPalette,
} from '../appearance';
import { h } from '../dom';
import { icon } from '../icons';
import { store } from '../state';
import type { FsListing, WallpaperEntry } from '../types';
import { card, notice, pageHeader, thumbUrl } from './shared';

export function wallpaperPage(el: HTMLElement): () => void {
  const note = notice();
  const appearanceCtl = appearanceControls();
  const grid = h('div', { class: 'wp-grid' });
  const extra: WallpaperEntry[] = [];
  let entries: WallpaperEntry[] = [];
  let paletteTheme: 'dark' | 'light' = appearance.theme;
  const paletteGrid = h('div', { class: 'palette-grid' });
  const paletteTabs = h('div', { class: 'palette-tabs' });
  const presetRow = h('div', { class: 'swatch-row' });
  const savedRow = h('div', { class: 'swatch-row' });
  const saveName = h('input', { type: 'text', class: 'grow', placeholder: 'Name these colours', spellcheck: 'false', maxlength: '40' }) as HTMLInputElement;

  /** A small three-colour chip standing in for a whole palette. */
  const chip = (p: { background: string; surface: string; accent: string }) =>
    h('span', { class: 'swatch-chip', style: `background:${p.background}` },
      h('i', { style: `background:${p.surface}` }), h('i', { style: `background:${p.accent}` }));

  const renderSchemes = () => {
    presetRow.replaceChildren(...PRESETS.map((preset) => {
      const on = appearance.preset === preset.id;
      const b = h('button', { class: `swatch${on ? ' on' : ''}`, type: 'button', title: preset.note },
        chip(preset[paletteTheme]), h('span', {}, preset.name));
      b.onclick = () => usePreset(preset.id);
      return b;
    }));
    const saved = savedPalettes();
    savedRow.replaceChildren(...saved.map((s) => {
      const b = h('button', { class: 'swatch', type: 'button', title: `Use “${s.name}”` }, chip(s[paletteTheme]), h('span', {}, s.name));
      b.onclick = () => useSavedPalette(s.name);
      const del = h('button', { class: 'swatch-del', type: 'button', title: `Forget “${s.name}”`, 'aria-label': `Forget ${s.name}` }, icon('close', 12));
      del.onclick = () => deleteSavedPalette(s.name);
      return h('span', { class: 'swatch-wrap' }, b, del);
    }));
    if (!saved.length) savedRow.replaceChildren(h('span', { class: 'row-help' }, 'Nothing saved yet. Tune the colours below, then keep them under a name.'));
    saveName.placeholder = appearance.preset ? `Name a change to ${PRESETS.find((x) => x.id === appearance.preset)?.name ?? 'these colours'}` : 'Name these colours';
  };

  const keep = () => {
    const name = saveName.value.trim();
    if (!name) return note.show('Give the palette a name first.', 'info');
    savePalette(name);
    saveName.value = '';
    note.show(`Saved “${name}”.`, 'ok');
  };
  saveName.addEventListener('keydown', (e) => { if (e.key === 'Enter') keep(); });

  const renderPalette = () => {
    const p = paletteFor(paletteTheme);
    paletteGrid.replaceChildren();
    const fields: Array<[keyof typeof p, string]> = [
      ['accent', 'Highlight'], ['background', 'Background'], ['surface', 'Surface'],
      ['surfaceStrong', 'Raised surface'], ['border', 'Border'], ['text', 'Text'], ['muted', 'Muted text'],
    ];
    for (const [key, label] of fields) {
      const input = h('input', { type: 'color', value: p[key], 'aria-label': `${label} color` }) as HTMLInputElement;
      input.addEventListener('input', () => setPaletteColor(paletteTheme, key, input.value));
      paletteGrid.appendChild(h('label', { class: 'palette-field' }, h('span', {}, label), input, h('code', {}, p[key].toUpperCase())));
    }
    paletteTabs.replaceChildren(...(['dark', 'light'] as const).map((theme) => {
      const button = h('button', { class: `btn${paletteTheme === theme ? ' primary' : ''}`, type: 'button' }, theme === 'dark' ? 'Dark theme' : 'Light theme');
      button.onclick = () => { paletteTheme = theme; renderPalette(); };
      return button;
    }));
    renderSchemes();
  };
  renderPalette();

  const folderIn = h('input', { type: 'text', class: 'grow', placeholder: '~/Pictures/Wallpapers', spellcheck: 'false' }) as HTMLInputElement;
  const addBtn = h('button', { class: 'btn', onclick: () => addFolder(folderIn.value.trim()) }, icon('folder', 14), 'Add pictures from this folder');
  folderIn.addEventListener('keydown', (e) => {
    if (e.key === 'Enter') addFolder(folderIn.value.trim());
  });

  el.append(
    pageHeader('Appearance', 'The colours of the desktop and the picture behind them. Images in ~/Pictures/Wallpapers and /usr/share/backgrounds are listed here.'),
    note.el,
    card('Theme', appearanceCtl.el, h('p', { class: 'row-help' }, 'Light or dark, and the colours each one uses. Changes apply at once, across the whole desktop.')),
    card('Colour scheme', paletteTabs, presetRow,
      h('h3', { class: 'card-sub' }, 'Your palettes'), savedRow,
      h('div', { class: 'inline-form' }, saveName, h('button', { class: 'btn', type: 'button', onclick: keep }, icon('check', 14), 'Save these colours'))),
    card('Colours', paletteGrid, h('p', { class: 'row-help' }, 'Seven colours make up the whole desktop: everything else is worked out from them.')),
    card(null, grid),
    card('Add a folder', h('div', { class: 'inline-form' }, folderIn, addBtn), h('div', { class: 'row-help' }, 'An image can also be set by right-clicking it in Files or Image Viewer and choosing “Set as Background”.')),
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
    grid.appendChild(tile('MindOS Graphite', 'Static graphite wallpaper', h('span', { class: 'wallpaper builtin' }), wp.mode !== 'image' || !wp.path, () => choose(null)));
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
    if (!all.length) grid.appendChild(h('div', { class: 'row-help' }, 'No images found. Add images to ~/Pictures/Wallpapers, or add a folder below.'));
  };

  const addFolder = (path: string) => {
    if (!path) return;
    bridge
      .call<FsListing>('fs.list', { path, hidden: false })
      .then((l) => {
        const images = l.entries.filter((e) => e.image && !e.dir);
        if (!images.length) return note.show('No images found in that folder.', 'info');
        const folder = l.path.split('/').filter(Boolean).pop() ?? l.path;
        for (const e of images) if (!extra.some((x) => x.path === e.path)) extra.push({ path: e.path, name: e.name.replace(/\.[^.]+$/, ''), folder });
        note.show(`Added ${images.length} image${images.length === 1 ? '' : 's'} from ${l.path}.`, 'ok');
        render();
      })
      .catch((e) => note.show(bridge.reason(e), 'error'));
  };

  render();
  bridge
    .call<WallpaperEntry[]>('wallpaper.list')
    .then((list) => {
      entries = Array.isArray(list) ? list : [];
      render();
    })
    .catch((e) => note.show(`Could not list wallpapers: ${bridge.reason(e)}`, 'error'));
  store.bind(grid, 'layout', render);
  const unlisten = onAppearance(renderPalette);
  return () => { appearanceCtl.destroy(); unlisten(); };
}
