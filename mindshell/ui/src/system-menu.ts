import * as bridge from './bridge';
import { h } from './dom';
import { icon } from './icons';
import { armable } from './popups/shared';

const ACTIONS = [
  { action: 'lock', label: 'Lock', glyph: 'lock' },
  { action: 'logout', label: 'Log out', glyph: 'logout' },
  { action: 'suspend', label: 'Suspend', glyph: 'suspend' },
  { action: 'reboot', label: 'Restart', glyph: 'reboot' },
  { action: 'shutdown', label: 'Shut down', glyph: 'power' },
] as const;

export function systemControls(): { el: HTMLElement; destroy: () => void } {
  const menu = h('div', { class: 'system-menu' });
  const trigger = h('button', { class: 'btn system-menu-trigger', 'aria-label': 'Open system menu', 'aria-expanded': 'false', title: 'System menu' }, icon('power', 17));
  const panel = h('div', { class: 'system-menu-panel', hidden: true, role: 'menu', 'aria-label': 'System menu' });
  const setOpen = (open: boolean) => {
    panel.hidden = !open;
    trigger.setAttribute('aria-expanded', String(open));
    menu.classList.toggle('open', open);
  };
  for (const [mode, label] of [['area', 'Screenshot area'], ['screen', 'Screenshot screen']]) {
    panel.append(h('button', { class: 'system-menu-item', role: 'menuitem', onclick: () => {
      setOpen(false);
      setTimeout(() => bridge.send('shell.exec', { cmd: 'mindos-screenshot ' + mode }), 250);
    } }, icon('image', 16), label));
  }
  for (const action of ACTIONS) {
    const label = h('span', {}, action.label);
    const button = h('button', { class: `system-menu-item system-menu-${action.action}`, role: 'menuitem' }, icon(action.glyph, 16), label);
    const fire = () => {
      if (action.action === 'lock') bridge.send('lock.now');
      else bridge.send('system.power', { action: action.action });
      setOpen(false);
    };
    if (action.action === 'lock') button.addEventListener('click', fire);
    else armable(button, label, action.label, 'Confirm', fire);
    panel.append(button);
  }
  trigger.addEventListener('click', () => setOpen(panel.hidden !== false));
  const outside = (event: PointerEvent) => { if (!menu.contains(event.target as Node)) setOpen(false); };
  const escape = (event: KeyboardEvent) => { if (event.key === 'Escape') setOpen(false); };
  document.addEventListener('pointerdown', outside);
  document.addEventListener('keydown', escape);
  menu.append(trigger, panel);
  return { el: menu, destroy: () => { document.removeEventListener('pointerdown', outside); document.removeEventListener('keydown', escape); } };
}
