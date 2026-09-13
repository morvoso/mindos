// The space switch in the top bar: one recessed track with a stop for every
// space and a lit thumb that slides onto the live one. Spaces have names of
// any length, so the thumb is measured onto its stop -- position and width
// both spring -- instead of assuming stops of equal size.
import { h } from './dom';
import { icon } from './icons';
import { activeSpace, spaceGlyph, spaces, switchSpace } from './spaces';
import { store } from './state';

export function spaceSwitch(onEdit: () => void): { el: HTMLElement; destroy: () => void } {
  const thumb = h('span', { class: 'space-switch-thumb', 'aria-hidden': 'true' });
  const track = h('div', { class: 'space-switch-track', role: 'radiogroup', 'aria-label': 'Space' }, thumb);
  const edit = h('button', { class: 'space-switch-edit', type: 'button', title: 'Edit spaces', 'aria-label': 'Edit spaces', onclick: onEdit }, icon('plus', 13));
  const el = h('div', { class: 'space-switch' }, track, edit);
  let buttons: HTMLButtonElement[] = [];
  let current = activeSpace().id;

  /** Put the thumb on the live stop. `animate` false jumps (first paint, a resize). */
  const place = (animate: boolean) => {
    const b = buttons.find((x) => x.dataset.space === current);
    if (!b || !b.offsetWidth) return;
    if (!animate) thumb.style.transition = 'none';
    thumb.style.width = `${b.offsetWidth}px`;
    thumb.style.transform = `translateX(${b.offsetLeft - 3}px)`;
    if (!animate) { void thumb.offsetWidth; thumb.style.transition = ''; }
  };
  const mark = () => {
    for (const b of buttons) {
      const on = b.dataset.space === current;
      b.setAttribute('aria-checked', String(on));
      b.tabIndex = on ? 0 : -1;
    }
  };
  const choose = (id: string) => {
    // Slide now; the layout round-trip that changes the desktop lands after.
    current = id; mark(); place(true);
    switchSpace(id);
  };

  let listKey = '';
  const render = () => {
    const list = spaces();
    const key = JSON.stringify(list.map((s) => [s.id, s.name, s.icon]));
    const active = activeSpace().id;
    if (key === listKey) {
      if (active !== current) { current = active; mark(); place(true); }
      return;
    }
    listKey = key; current = active;
    buttons = list.map((s) => h('button', {
      class: 'space-opt', type: 'button', role: 'radio', title: s.name, dataset: { space: s.id },
      onclick: () => choose(s.id),
    }, spaceGlyph(s), h('span', { class: 'space-opt-label' }, s.name)));
    track.replaceChildren(thumb, ...buttons);
    mark();
    // A renamed or added space moves the stops under the thumb: it follows
    // them rather than jumping.
    requestAnimationFrame(() => place(false));
  };
  // A radio group moves with the arrow keys, not with tab.
  track.addEventListener('keydown', (e) => {
    if (!['ArrowLeft', 'ArrowUp', 'ArrowRight', 'ArrowDown'].includes(e.key) || !buttons.length) return;
    e.preventDefault();
    const step = e.key === 'ArrowLeft' || e.key === 'ArrowUp' ? buttons.length - 1 : 1;
    const next = buttons[(buttons.findIndex((b) => b.dataset.space === current) + step) % buttons.length];
    next.focus();
    choose(next.dataset.space!);
  });
  // Labels come and go with the width of the bar, and fonts load late.
  const resize = new ResizeObserver(() => place(false));
  resize.observe(track);
  render();
  const off = store.on('layout', render);
  return { el, destroy: () => { off(); resize.disconnect(); } };
}
