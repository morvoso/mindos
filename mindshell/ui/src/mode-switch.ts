// The desktop's mode switch, the one control in the top bar that changes what
// the whole workspace is for. It reads as a single object -- one recessed
// track, two stops, a lit thumb that slides onto the live mode -- rather than
// as two buttons that happen to sit next to each other.
import { h } from './dom';
import { icon } from './icons';

export type WorkspaceMode = 'gaming' | 'productivity';

const MODES: { id: WorkspaceMode; label: string; glyph: string; hint: string }[] = [
  { id: 'gaming', label: 'Gaming', glyph: 'gamepad', hint: 'Your library, performance and games front and centre' },
  { id: 'productivity', label: 'Work', glyph: 'grid', hint: 'Shortcuts, files and notes front and centre' },
];

/**
 * Two stops on one track. `pick` is called with the mode the user asked for.
 * Choosing a mode rebuilds the whole workspace, switch included, so `from`
 * says which stop the thumb should start on: a new switch that opens on the
 * old mode and flips a frame later carries the slide across the rebuild.
 */
export function modeSwitch(current: WorkspaceMode, pick: (mode: WorkspaceMode) => void, from?: WorkspaceMode): HTMLElement {
  const el = h('div', { class: 'mode-switch', role: 'radiogroup', 'aria-label': 'Desktop mode', dataset: { mode: from ?? current } },
    h('span', { class: 'mode-switch-thumb', 'aria-hidden': 'true' }));
  if (from && from !== current) requestAnimationFrame(() => requestAnimationFrame(() => { el.dataset.mode = current; }));
  const choose = (mode: WorkspaceMode) => {
    // Slide now; the layout round-trip that rebuilds the workspace lands after.
    el.dataset.mode = mode;
    for (const b of buttons) {
      const on = b.dataset.mode === mode;
      b.setAttribute('aria-checked', String(on));
      b.tabIndex = on ? 0 : -1;
    }
    pick(mode);
  };
  const buttons = MODES.map((m) => h('button', {
    class: 'mode-opt', type: 'button', role: 'radio', 'aria-checked': String(m.id === current),
    tabindex: m.id === current ? 0 : -1, title: `${m.label} — ${m.hint}`, dataset: { mode: m.id },
    onclick: () => choose(m.id),
  }, icon(m.glyph, 15), h('span', { class: 'mode-opt-label' }, m.label)));
  el.append(...buttons);
  // A radio group moves with the arrow keys, not with tab.
  el.addEventListener('keydown', (e) => {
    if (!['ArrowLeft', 'ArrowUp', 'ArrowRight', 'ArrowDown'].includes(e.key)) return;
    e.preventDefault();
    const step = e.key === 'ArrowLeft' || e.key === 'ArrowUp' ? MODES.length - 1 : 1;
    const next = MODES[(MODES.findIndex(m => m.id === el.dataset.mode) + step) % MODES.length];
    buttons[MODES.indexOf(next)].focus();
    choose(next.id);
  });
  return el;
}
