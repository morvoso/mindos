// The system menu: session, power and screenshots. Destructive entries arm
// on the first click and say so, so nothing here ever happens by accident.

import * as bridge from '../bridge';
import { h } from '../dom';
import { icon } from '../icons';
import { store } from '../state';
import type { PopupContent, PopupCtx } from './shared';

export const POWER_ACTIONS = [
  { action: 'lock', label: 'Lock', icon: 'lock', note: 'Keep everything running' },
  { action: 'logout', label: 'Log out', icon: 'logout', note: 'Close this session' },
  { action: 'suspend', label: 'Sleep', icon: 'suspend', note: 'Low power, wake instantly' },
  { action: 'reboot', label: 'Restart', icon: 'reboot', note: 'Close everything and boot again' },
  { action: 'shutdown', label: 'Shut down', icon: 'power', note: 'Close everything and power off' },
] as const;

const SHOTS = [
  { mode: 'area', label: 'Screenshot an area' },
  { mode: 'screen', label: 'Screenshot the screen' },
] as const;

export function powerPopup(ctx: PopupCtx): PopupContent {
  const list = h('div', { class: 'sysmenu-list' });
  for (const a of POWER_ACTIONS) {
    const note = h('span', { class: 'sysmenu-note' }, a.note);
    const row = h('button', { class: `sysmenu-item sysmenu-${a.action}`, role: 'menuitem' },
      h('span', { class: 'sysmenu-ic' }, icon(a.icon, 18)),
      h('span', { class: 'sysmenu-text' }, h('strong', {}, a.label), note));
    if (a.action === 'lock') {
      // Nothing is lost by locking, so it goes on the first click.
      row.addEventListener('click', () => { bridge.send('lock.now'); ctx.close(); });
    } else {
      let armed = false;
      let timer: ReturnType<typeof setTimeout> | undefined;
      row.addEventListener('click', () => {
        if (armed) {
          if (timer) clearTimeout(timer);
          bridge.send('system.power', { action: a.action });
          ctx.close();
          return;
        }
        for (const other of list.querySelectorAll('.sysmenu-item.armed')) other.dispatchEvent(new CustomEvent('disarm'));
        armed = true;
        row.classList.add('armed');
        note.textContent = 'Click again to confirm';
        timer = setTimeout(() => row.dispatchEvent(new CustomEvent('disarm')), 4000);
      });
      row.addEventListener('disarm', () => {
        if (timer) clearTimeout(timer);
        armed = false;
        row.classList.remove('armed');
        note.textContent = a.note;
      });
    }
    list.appendChild(row);
  }
  list.appendChild(h('div', { class: 'menu-sep' }));
  for (const s of SHOTS) {
    const row = h('button', { class: 'sysmenu-item sysmenu-shot', role: 'menuitem' },
      h('span', { class: 'sysmenu-ic' }, icon('image', 18)),
      h('span', { class: 'sysmenu-text' }, h('strong', {}, s.label)));
    // Let the popup disappear before the capture starts, or it is in the shot.
    row.addEventListener('click', () => {
      ctx.close();
      setTimeout(() => bridge.send('shell.exec', { cmd: `mindos-screenshot ${s.mode}` }), 250);
    });
    list.appendChild(row);
  }
  const who = store.state.user ? `${store.state.user}@${store.state.host}` : 'Session';
  const el = h('div', { class: 'pop-body sysmenu' }, h('div', { class: 'pop-title' }, who), list);
  return { el, w: 288 };
}
