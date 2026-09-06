import * as bridge from '../bridge';
import { h } from '../dom';
import { icon } from '../icons';
import { armable, type PopupContent, type PopupCtx } from './shared';

export const POWER_ACTIONS = [
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
    armable(btn, label, a.label, 'Confirm', () => {
      bridge.send('system.power', { action: a.action });
      ctx.close();
    });
    row.appendChild(btn);
  }
  const el = h('div', { class: 'pop-body power' }, h('div', { class: 'pop-title' }, 'POWER'), row, h('div', { class: 'pop-hint' }, 'Click twice to confirm'));
  return { el, w: 352 };
}
