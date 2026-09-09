// Picking an application: a searchable grid of everything installed. Used by
// the productivity desktop to add shortcuts, and by anything else that needs
// the user to name an app. Clicking a tile toggles it, so several can go in
// without reopening the picker.

import { h } from '../dom';
import { icon } from '../icons';
import type { AppInfo, WorkspaceShortcut } from '../types';
import type { PopupContent, PopupCtx } from './shared';

/** A sensible glyph for the left menu when the app has no icon file. */
function glyphFor(a: AppInfo): string {
  const s = `${a.id} ${a.categories.join(' ')}`.toLowerCase();
  if (/web|browser|firefox|chrom/.test(s)) return 'globe';
  if (/mail|thunderbird|evolution|geary/.test(s)) return 'mail';
  if (/game|steam|lutris|heroic/.test(s)) return 'gamepad';
  if (/term|console|kitty|foot/.test(s)) return 'terminal';
  if (/office|document|writer|text|editor/.test(s)) return 'edit';
  if (/file|nautilus|manager/.test(s)) return 'folder';
  if (/audio|video|music|player|media/.test(s)) return 'music';
  if (/setting|config|control|prefer/.test(s)) return 'gear';
  return 'box';
}

export function appPickerPopup(ctx: PopupCtx): PopupContent {
  const store = ctx.store;
  const search = h('input', { class: 'input', type: 'search', placeholder: 'Search applications…', 'aria-label': 'Search applications' }) as HTMLInputElement;
  const grid = h('div', { class: 'picker-grid' });
  const empty = h('p', { class: 'pop-hint' }, 'No application matches that.');

  const shortcuts = (): WorkspaceShortcut[] => store.state.layout.desktop.workspace?.shortcuts ?? [];
  const has = (id: string) => shortcuts().some((s) => s.appId === id);

  const toggle = (a: AppInfo) => {
    void store.updateLayout((l) => {
      const w = l.desktop.workspace ?? { mode: 'productivity' as const, notes: '' };
      const list = w.shortcuts ?? [];
      w.shortcuts = list.some((s) => s.appId === a.id)
        ? list.filter((s) => s.appId !== a.id)
        : [...list, { id: `app-${a.id}`, appId: a.id, label: a.name, icon: glyphFor(a) }];
      l.desktop.workspace = w;
    });
    render();
  };

  const tile = (a: AppInfo) => {
    const on = has(a.id);
    const art = a.icon
      ? h('img', { src: a.icon, alt: '', width: 40, height: 40, loading: 'lazy', onerror: (e: Event) => (e.target as HTMLElement).replaceWith(icon(glyphFor(a), 40)) })
      : icon(glyphFor(a), 40);
    return h('button', { class: `picker-item${on ? ' on' : ''}`, title: a.comment || a.name, onclick: () => toggle(a) },
      h('span', { class: 'picker-art' }, art, ...(on ? [h('span', { class: 'picker-check' }, icon('check', 12))] : [])),
      h('span', { class: 'picker-name' }, a.name));
  };

  const render = () => {
    const q = search.value.trim().toLowerCase();
    const list = store.state.apps
      .filter((a) => !q || `${a.name} ${a.comment ?? ''} ${a.categories.join(' ')}`.toLowerCase().includes(q))
      .sort((a, b) => a.name.localeCompare(b.name))
      .slice(0, 120);
    grid.replaceChildren(...list.map(tile));
    empty.hidden = list.length > 0;
  };
  render();
  search.addEventListener('input', render);
  search.addEventListener('keydown', (e) => {
    // Enter takes the first match, so the keyboard alone can add a shortcut.
    if (e.key !== 'Enter') return;
    const first = grid.firstElementChild as HTMLElement | null;
    first?.click();
  });

  const el = h('div', { class: 'pop-body picker' },
    h('div', { class: 'pop-head' },
      h('span', { class: 'pop-title' }, 'Add a shortcut'),
      h('span', { class: 'lch-gap' }),
      h('button', { class: 'tool', title: 'Close', onclick: () => ctx.close() }, icon('x', 14))),
    search, grid, empty,
    h('p', { class: 'pop-hint' }, 'Click an application to add or remove its shortcut.'));
  return { el, w: 520, h: 470, focus: () => search.focus() };
}
