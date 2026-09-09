// The window layout mode switch: shows the compositor's current mode next to
// the clock; click for the chooser, middle-click to cycle (Super+T).

import * as bridge from '../bridge';
import { h } from '../dom';
import { icon } from '../icons';
import { registerWidget } from './registry';
import { panelItem } from './common';

export interface ModeInfo {
  name: string;
  label: string;
  icon: string;
  like: string;
  blurb: string;
  hint: string;
}

/** The three modes mindwm offers, in the order Super+T cycles them. */
export const MODES: ModeInfo[] = [
  {
    name: 'floating',
    label: 'Floating',
    icon: 'mode-floating',
    like: 'like KDE',
    blurb: 'Windows open at their own position and can overlap. Drag them by the title bar, maximise or snap them.',
    hint: 'Drag a window edge to resize · Super+drag move · Super+right-drag resize · Super+R size · Super+Shift+arrows snap',
  },
  {
    name: 'dwindle',
    label: 'Tiles',
    icon: 'mode-tiles',
    like: 'like Hyprland',
    blurb: 'Every window occupies a tile. Each new window splits the focused tile in half. Windows do not overlap.',
    hint: 'Drag the gap to move a split · Super+drag move a tile · Super+right-drag the split · Super+R size · Super+Shift+F float',
  },
  {
    name: 'columns',
    label: 'Columns',
    icon: 'mode-columns',
    like: 'like Niri',
    blurb: 'Windows are arranged in columns on a horizontally scrolling strip. Suited to ultrawide displays.',
    hint: 'Drag the gap to set a width · Super+drag move a column · Super+right-drag its width · Super+R width',
  },
];

export function modeInfo(name: string | undefined): ModeInfo {
  return MODES.find((m) => m.name === name) ?? { name: name ?? '', label: name ? name[0].toUpperCase() + name.slice(1) : 'Layout', icon: 'layout', like: '', blurb: '', hint: '' };
}

registerWidget({
  type: 'layout-mode',
  name: 'Window layout',
  description: 'Shows how windows are arranged (floating, tiles or columns). Click to change it, middle-click to cycle.',
  icon: 'layout',
  containers: ['panel'],
  defaults: { label: false },
  settings: { label: { label: 'Show the mode name', type: 'boolean', help: 'Floating, Tiles or Columns next to the icon' } },
  create(ctx) {
    const el = panelItem(ctx, 'w-layout-mode');
    const ic = h('span', { class: 'w-ic' });
    const label = h('span', { class: 'w-label' });
    el.append(ic, label);
    let cfg = ctx.config;
    const render = () => {
      const info = modeInfo(ctx.store.layoutMode?.mode);
      ic.replaceChildren(icon(info.icon, 18));
      label.textContent = info.label;
      label.hidden = !cfg.label || !!ctx.panel?.vertical;
      el.title = `Window layout: ${info.label}${info.like ? ` (${info.like})` : ''} · click to change · Super+T cycles`;
      el.dataset.mode = info.name;
    };
    render();
    if (!ctx.store.layoutMode) void ctx.store.fetchLayoutMode();
    ctx.store.bind(el, 'layoutMode', render);
    ctx.store.bind(el, 'popups', () => el.classList.toggle('open', ctx.store.popups.has('layout-mode')));
    el.addEventListener('click', () => ctx.togglePopup('layout-mode', {}, { anchor: ctx.anchorOf(el) }));
    el.addEventListener('auxclick', (e) => {
      if (e.button === 1) bridge.send('wm.cycleLayoutMode');
    });
    return {
      el,
      update(c) {
        cfg = c;
        render();
      },
    };
  },
});
