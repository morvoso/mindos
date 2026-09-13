// Settings › Desktop › Spaces: make, name, order and dress the spaces, and
// choose which apps stay open on all of them.

import { PRESETS, savedPalettes } from '../appearance';
import { h } from '../dom';
import { icon } from '../icons';
import { PERF_MODES } from '../perf';
import { addSpace, moveSpace, removeSpace, setSticky, SPACE_ICONS, spaceGlyph, spaces, updateSpace, workspaceOf } from '../spaces';
import { store } from '../state';
import type { PerfMode, Space } from '../types';
import { MODES } from '../widgets/layout-mode';
import { card, row, selectBox, toggle } from './shared';

export function spacesCard(): { el: HTMLElement; destroy: () => void } {
  const list = h('div', { class: 'spaces-list' });
  const add = h('button', { class: 'btn', onclick: () => { const s = addSpace(`Space ${spaces().length + 1}`); openEditor = s.id; } }, icon('plus', 14), 'Add space');
  const sticky = h('div', { class: 'spaces-sticky' });
  const el = h('div', {},
    card('Spaces',
      h('p', { class: 'card-help' }, 'Each space is its own desktop with its own windows, shortcuts and notes. It can also set the performance mode, the colours and the window layout when you move to it. Switch spaces from the top bar.'),
      list,
      h('div', { class: 'card-actions' }, add)),
    card('On every space',
      h('p', { class: 'card-help' }, 'Windows of these apps stay open whichever space you are on. Add one by right-clicking its window in the task bar and choosing Show on all spaces.'),
      sticky));

  /** The space whose details are unfolded; one at a time keeps the list short. */
  let openEditor = '';
  let picking = '';

  const editor = (s: Space) => {
    const perf = selectBox<string>([{ value: '', label: 'Leave as it is' }, ...PERF_MODES.map((m) => ({ value: m.mode, label: m.label }))], s.perf ?? '',
      (v) => void updateSpace(s.id, (x) => { if (v) x.perf = v as PerfMode; else delete x.perf; }));
    const palette = selectBox<string>([
      { value: '', label: 'Appearance colours' },
      ...PRESETS.map((p) => ({ value: `preset:${p.id}`, label: p.name })),
      ...savedPalettes().map((p) => ({ value: `saved:${p.name}`, label: `${p.name} (saved)` })),
    ], s.palette ?? '', (v) => void updateSpace(s.id, (x) => { if (v) x.palette = v; else delete x.palette; }));
    const layout = selectBox<string>([{ value: '', label: 'Leave as it is' }, ...MODES.map((m) => ({ value: m.name, label: `${m.label} (${m.like})` }))], s.layoutMode ?? '',
      (v) => void updateSpace(s.id, (x) => { if (v) x.layoutMode = v; else delete x.layoutMode; }));
    const recent = toggle(!!s.recent, (v) => void updateSpace(s.id, (x) => { x.recent = v; }));
    return h('div', { class: 'space-editor' },
      row('Performance', 'The performance mode to switch to on entering this space.', perf),
      row('Colours', 'A palette for this space. Appearance colours follows Settings › Appearance.', palette),
      row('Window layout', 'How windows are arranged on this space.', layout),
      row('Resume playing', 'Show recently played games and a link to the Game Library on this desktop.', recent));
  };

  const item = (s: Space, i: number, all: Space[]) => {
    const name = h('input', { class: 'input space-name', value: s.name, maxlength: 40, 'aria-label': 'Space name' }) as HTMLInputElement;
    const commit = () => {
      const v = name.value.trim();
      if (!v) { name.value = s.name; return; }
      if (v !== s.name) void updateSpace(s.id, (x) => { x.name = v; });
    };
    name.addEventListener('change', commit);
    name.addEventListener('keydown', (e) => { if (e.key === 'Enter') name.blur(); });
    const glyph = h('button', {
      class: 'space-icon-btn', title: 'Choose an icon', 'aria-expanded': String(picking === s.id),
      onclick: () => { picking = picking === s.id ? '' : s.id; render(true); },
    }, spaceGlyph(s, 18));
    const active = workspaceOf().space === s.id;
    const head = h('div', { class: 'space-item-head' },
      glyph, name,
      active ? h('span', { class: 'pill' }, 'Current') : null,
      h('span', { class: 'strip-gap' }),
      h('button', { class: 'tool', title: 'Move up', 'aria-label': 'Move up', disabled: i === 0, onclick: () => moveSpace(s.id, -1) }, icon('chevron-up', 14)),
      h('button', { class: 'tool', title: 'Move down', 'aria-label': 'Move down', disabled: i === all.length - 1, onclick: () => moveSpace(s.id, 1) }, icon('chevron-down', 14)),
      h('button', {
        class: 'tool', title: 'Settings for this space', 'aria-label': 'Settings for this space', 'aria-expanded': String(openEditor === s.id),
        onclick: () => { openEditor = openEditor === s.id ? '' : s.id; render(true); },
      }, icon('sliders', 14)),
      h('button', {
        class: 'tool danger', title: all.length < 2 ? 'The last space cannot be removed' : 'Remove space', 'aria-label': 'Remove space', disabled: all.length < 2,
        onclick: () => { if (confirm(`Remove ${s.name}? Its windows move to another space; its shortcuts and notes are deleted.`)) removeSpace(s.id); },
      }, icon('trash', 14)));
    const picker = picking === s.id ? h('div', { class: 'space-icon-grid', role: 'listbox', 'aria-label': 'Icons' },
      h('button', { class: `space-icon-opt${s.icon ? '' : ' on'}`, title: 'No icon', onclick: () => { picking = ''; void updateSpace(s.id, (x) => { delete x.icon; }); } }, h('span', { class: 'space-initial' }, (s.name[0] ?? '?').toUpperCase())),
      ...SPACE_ICONS.map((g) => h('button', { class: `space-icon-opt${s.icon === g ? ' on' : ''}`, title: g, onclick: () => { picking = ''; void updateSpace(s.id, (x) => { x.icon = g; }); } }, icon(g, 18)))) : null;
    return h('div', { class: `space-item${active ? ' active' : ''}` }, head, picker, openEditor === s.id ? editor(s) : null);
  };

  let key = '';
  const render = (force = false) => {
    const w = workspaceOf();
    const next = JSON.stringify([w, savedPalettes().map((p) => p.name)]);
    if (!force && next === key) return;
    // Don't rebuild under someone typing a name.
    if (!force && list.contains(document.activeElement) && document.activeElement instanceof HTMLInputElement) return;
    key = next;
    const all = w.spaces;
    list.replaceChildren(...all.map((s, i) => item(s, i, all)));
    const apps = w.sticky ?? [];
    sticky.replaceChildren(apps.length
      ? h('div', { class: 'spaces-sticky-list' }, ...apps.map((id) => {
        const app = store.state.apps.find((a) => a.id.replace(/\.desktop$/, '').toLowerCase() === id.toLowerCase());
        return h('span', { class: 'pill spaces-sticky-app' }, app?.name ?? id,
          h('button', { class: 'tool', title: `Keep ${app?.name ?? id} on one space`, 'aria-label': `Remove ${app?.name ?? id}`, onclick: () => void setSticky(id, false) }, icon('x', 12)));
      }))
      : h('p', { class: 'row-help' }, 'No apps yet.'));
  };
  render(true);
  const offs = [store.on('layout', () => render()), store.on('apps', () => render(true))];
  return { el, destroy: () => offs.forEach((off) => off()) };
}
