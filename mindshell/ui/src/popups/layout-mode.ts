// The window layout chooser: one row per mode with what it is like and the
// keys that matter in it.

import * as bridge from '../bridge';
import { h } from '../dom';
import { icon } from '../icons';
import { MODES } from '../widgets/layout-mode';
import type { PopupContent, PopupCtx } from './shared';

export function layoutModePopup(ctx: PopupCtx): PopupContent {
  const list = h('div', { class: 'lm-list' });
  const el = h(
    'div',
    { class: 'pop-body lm' },
    h('div', { class: 'pop-head' }, h('span', { class: 'set-ic' }, icon('layout', 16)), h('span', { class: 'pop-title' }, 'WINDOW LAYOUT')),
    list,
    h(
      'div',
      { class: 'pop-actions' },
      h('span', { class: 'pop-hint lm-hint' }, 'Super+T cycles · the choice is remembered'),
      h('span', { class: 'strip-gap' }),
      h('button', { class: 'btn small', onclick: () => {
        bridge.send('shell.openApp', { name: 'settings', page: 'displays' });
        ctx.close();
      } }, icon('display', 14), 'Displays'),
    ),
  );
  const available = () => {
    const names = ctx.store.layoutMode?.modes?.map((m) => m.name);
    return names && names.length ? MODES.filter((m) => names.includes(m.name)) : MODES;
  };
  const render = () => {
    const current = ctx.store.layoutMode?.mode;
    list.replaceChildren(
      ...available().map((m) => {
        const btn = h(
          'button',
          { class: `lm-item${m.name === current ? ' on' : ''}` },
          h('span', { class: 'lm-ic' }, icon(m.icon, 22)),
          h(
            'span',
            { class: 'lm-text' },
            h('span', { class: 'lm-name' }, h('b', {}, m.label), h('span', { class: 'lm-like' }, m.like)),
            h('span', { class: 'lm-blurb' }, m.blurb),
            h('span', { class: 'lm-keys mono' }, m.hint),
          ),
          h('span', { class: 'lm-check' }, icon('check', 16)),
        );
        btn.addEventListener('click', () => {
          bridge.send('wm.setLayoutMode', { mode: m.name });
          ctx.close();
        });
        return btn;
      }),
    );
  };
  render();
  if (!ctx.store.layoutMode) void ctx.store.fetchLayoutMode();
  ctx.store.bind(list, 'layoutMode', render);
  return { el, w: 400 };
}
