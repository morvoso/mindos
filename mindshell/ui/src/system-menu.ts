// The system button in the desktop header. It opens the shell's `power`
// popup, which is its own overlay-layer window: it draws above the desktop,
// the shelf and any application window, and closes on Escape or a click
// outside. (An in-page dropdown could not do either — the desktop is the
// bottom layer, so its menu ended up under everything else.)

import * as actions from './actions';
import { h } from './dom';
import { icon } from './icons';
import { rectIn } from './geometry';
import { store } from './state';

export function systemControls(): { el: HTMLElement; destroy: () => void } {
  const trigger = h('button', { class: 'tool system-menu-trigger', 'aria-label': 'System menu', 'aria-haspopup': 'dialog', title: 'Power, session and screenshots' }, icon('power', 17));
  const menu = h('div', { class: 'system-menu' }, trigger);
  trigger.addEventListener('click', () => {
    const root = menu.closest<HTMLElement>('.win') ?? document.body;
    const r = rectIn(root, trigger);
    actions.togglePopup('power', {}, { anchor: { x: r.x, y: r.y, w: r.w, h: r.h, edge: 'top' } });
  });
  const sync = () => trigger.classList.toggle('on', store.popups.has('power'));
  store.bind(trigger, 'popups', sync);
  sync();
  return { el: menu, destroy: () => {} };
}
