import * as bridge from '../bridge';
import { h } from '../dom';
import { icon } from '../icons';
import { armable, type PopupContent, type PopupCtx } from './shared';

export const POWER_ACTIONS = [
  { action: 'lock', label: 'Lock', icon: 'lock' },
  { action: 'logout', label: 'Log out', icon: 'logout' },
  { action: 'suspend', label: 'Suspend', icon: 'suspend' },
  { action: 'reboot', label: 'Restart', icon: 'reboot' },
  { action: 'shutdown', label: 'Shut down', icon: 'power' },
] as const;

export function powerPopup(ctx: PopupCtx): PopupContent {
  const row = h('div', { class: 'power-row' });
  for (const a of POWER_ACTIONS) {
    const label = h('span', { class: 'power-label' }, a.label);
    const btn = h('button', { class: `power-btn power-${a.action}` }, h('span', { class: 'power-ic' }, icon(a.icon, 22)), label);
    if (a.action === 'lock') {
      // Nothing is lost by locking, so it goes on the first click.
      btn.addEventListener('click', () => {
        bridge.send('lock.now');
        ctx.close();
      });
    } else {
      armable(btn, label, a.label, 'Confirm', () => {
        bridge.send('system.power', { action: a.action });
        ctx.close();
      });
    }
    row.appendChild(btn);
  }
  const el = h('div', { class: 'pop-body power' }, h('div', { class: 'pop-title' }, 'POWER'), row, h('div', { class: 'pop-hint' }, 'Lock goes at once; the rest need a second click'));
  return { el, w: 420 };
}
