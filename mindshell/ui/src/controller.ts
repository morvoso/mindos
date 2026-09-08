import { h } from './dom';

/** Controller activation follows the same buttons/forms as pointer and keyboard. */
export function installController(): void {
  let previous = new Set<number>(), nextMove = 0, timer: ReturnType<typeof setInterval> | undefined;
  const visible = () => [...document.querySelectorAll<HTMLElement>('.play-osk button, button, input, select, textarea, a[href]')].filter(el => !el.closest('[hidden]') && el.getClientRects().length && !el.hasAttribute('disabled') && (!document.querySelector('.play-osk') || !!el.closest('.play-osk')));
  function focus(el?: HTMLElement): void { document.querySelector('.play-controller-focus')?.classList.remove('play-controller-focus'); el?.classList.add('play-controller-focus'); el?.focus(); el?.scrollIntoView({ block: 'nearest' }); }
  function move(dx: number, dy: number): void {
    const items = visible(), current = document.activeElement as HTMLElement;
    if (!items.includes(current)) { focus(items[0]); return; }
    const r = current.getBoundingClientRect(), x = r.x + r.width / 2, y = r.y + r.height / 2;
    const target = items.filter(el => el !== current).map(el => { const b = el.getBoundingClientRect(), ex = b.x + b.width / 2 - x, ey = b.y + b.height / 2 - y; return { el, along: ex * dx + ey * dy, cross: Math.abs(ex * dy - ey * dx) }; }).filter(p => p.along > 3).sort((a, b) => (a.along + a.cross * 3) - (b.along + b.cross * 3))[0];
    focus(target?.el);
  }
  function tick(): void {
    if (document.hidden || !document.hasFocus()) { previous.clear(); return; }
    const pad = Array.from(navigator.getGamepads?.() || []).find(p => p?.connected && p.mapping === 'standard');
    if (!pad) return;
    const pressed = new Set(pad.buttons.flatMap((b, i) => b.pressed ? [i] : []));
    const fresh = (id: number) => pressed.has(id) && !previous.has(id);
    const dx = pressed.has(14) || (pad.axes[0] || 0) < -.6 ? -1 : pressed.has(15) || (pad.axes[0] || 0) > .6 ? 1 : 0;
    const dy = pressed.has(12) || (pad.axes[1] || 0) < -.6 ? -1 : pressed.has(13) || (pad.axes[1] || 0) > .6 ? 1 : 0;
    if ((dx || dy) && performance.now() >= nextMove) { move(dx, dy); nextMove = performance.now() + 190; }
    if (fresh(0)) {
      const el = document.activeElement;
      if (el instanceof HTMLInputElement || el instanceof HTMLTextAreaElement) showKeyboard(el);
      else if (el instanceof HTMLSelectElement) { el.selectedIndex = (el.selectedIndex + 1) % el.options.length; el.dispatchEvent(new Event('change', { bubbles: true })); }
      else if (el instanceof HTMLElement && visible().includes(el)) el.click();
      else focus(visible()[0]);
    }
    if (fresh(1)) { document.querySelector('.play-osk')?.remove(); document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true })); }
    if (fresh(3)) focus(document.querySelector<HTMLInputElement>('input[type=search]') || visible()[0]);
    previous = pressed;
  }
  const start = () => { if (!timer) timer = setInterval(tick, 50); };
  window.addEventListener('gamepadconnected', start);
  window.addEventListener('gamepaddisconnected', () => { if (!Array.from(navigator.getGamepads?.() || []).some(Boolean) && timer) { clearInterval(timer); timer = undefined; } });
  if (Array.from(navigator.getGamepads?.() || []).some(Boolean)) start();
  window.addEventListener('pagehide', () => { if (timer) clearInterval(timer); });
}

export function showKeyboard(input: HTMLInputElement | HTMLTextAreaElement): void {
  if (document.querySelector('.play-osk') || input.disabled || input.readOnly) return;
  let shifted = false, symbols = false;
  const keyboard = h('div', { class: 'play-osk', role: 'dialog', 'aria-label': 'On-screen keyboard' });
  const edit = (s: string) => { const a = input.selectionStart ?? input.value.length, b = input.selectionEnd ?? a; input.value = input.value.slice(0, a) + s + input.value.slice(b); try { input.setSelectionRange(a + s.length, a + s.length); } catch { /* Number fields have no selection. */ } input.dispatchEvent(new Event('input', { bubbles: true })); };
  const close = () => { keyboard.remove(); input.focus(); };
  function render(): void {
    const chars = symbols ? '1234567890!@#$%^&*()-_=+[]{};:\'",.<>/?\\|`~' : '1234567890qwertyuiopasdfghjklzxcvbnm';
    keyboard.replaceChildren(...[...chars].map(c => h('button', { type: 'button', onclick: () => edit(shifted ? c.toUpperCase() : c) }, shifted ? c.toUpperCase() : c)),
      h('button', { type: 'button', onclick: () => { shifted = !shifted; render(); } }, 'Shift'), h('button', { type: 'button', onclick: () => { symbols = !symbols; render(); } }, symbols ? 'ABC' : '#+='),
      h('button', { type: 'button', onclick: () => edit(' ') }, 'Space'), h('button', { type: 'button', onclick: () => { const a = input.selectionStart ?? input.value.length, b = input.selectionEnd ?? a; if (a === b && a) input.setSelectionRange(a - 1, b); edit(''); } }, '⌫'),
      h('button', { type: 'button', onclick: close }, 'Done'), h('button', { type: 'button', onclick: () => { close(); input.form?.requestSubmit(); } }, 'Enter'));
    keyboard.querySelector('button')?.focus();
  }
  document.body.append(keyboard); render();
}
