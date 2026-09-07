// The toast window: a stack of new notifications in the top-right corner of
// the primary output. The host sizes the window to the stack (`toast.fit`)
// and hides it when the stack is empty. Toasts do not take focus.

import * as bridge from './bridge';
import { h } from './dom';
import { icon } from './icons';
import { noticeCard } from './notices';
import { notificationCard } from './popups/notifications';
import { store } from './state';
import type { MindNotice, Notification } from './types';

const DEFAULT_MS = 6500;
const MIND_MS = 12000;
const MAX = 4;

export function renderToasts(root: HTMLElement, _output: string): void {
  root.classList.add('toast-window');
  const stack = h('div', { class: 'toast-stack' });
  root.appendChild(stack);
  const timers = new Map<HTMLElement, ReturnType<typeof setTimeout>>();

  let lastW = 0;
  let lastH = 0;
  const fit = () => {
    const r = stack.getBoundingClientRect();
    const w = stack.childElementCount ? Math.ceil(r.width) : 0;
    const hh = stack.childElementCount ? Math.ceil(r.height) : 0;
    if (w === lastW && hh === lastH) return;
    lastW = w;
    lastH = hh;
    bridge.send('toast.fit', { w, h: hh });
  };
  if (typeof ResizeObserver === 'function') new ResizeObserver(fit).observe(stack);

  const remove = (el: HTMLElement) => {
    const t = timers.get(el);
    if (t) clearTimeout(t);
    timers.delete(el);
    if (!el.isConnected) return;
    el.classList.add('out');
    setTimeout(() => {
      el.remove();
      fit();
    }, 180);
  };
  const arm = (el: HTMLElement, ms: number) => {
    if (ms <= 0) return;
    const t = timers.get(el);
    if (t) clearTimeout(t);
    timers.set(el, setTimeout(() => remove(el), ms));
  };
  const add = (card: HTMLElement, ms: number) => {
    const wrap = h('div', { class: 'toast' }, card, h('button', { class: 'toast-x', title: 'Close', onclick: (e: Event) => { e.stopPropagation(); remove(wrap); } }, icon('x', 13)));
    wrap.addEventListener('pointerenter', () => {
      const t = timers.get(wrap);
      if (t) clearTimeout(t);
    });
    wrap.addEventListener('pointerleave', () => arm(wrap, Math.min(ms || DEFAULT_MS, 3000)));
    stack.prepend(wrap);
    while (stack.childElementCount > MAX) remove(stack.lastElementChild as HTMLElement);
    requestAnimationFrame(() => wrap.classList.add('in'));
    arm(wrap, ms);
    fit();
    return wrap;
  };

  const showNotification = (n: Notification) => {
    if (n.quiet) return;
    // A replacement updates the toast already on screen.
    for (const el of [...stack.children] as HTMLElement[]) {
      if (el.dataset.nid === String(n.id)) remove(el);
    }
    const ms = n.timeout > 0 ? n.timeout : n.timeout === 0 || n.urgency >= 2 ? 0 : DEFAULT_MS;
    const wrap = add(notificationCard(n, { compact: true, after: () => remove(wrap) }), ms);
    wrap.dataset.nid = String(n.id);
    wrap.classList.add(`u${n.urgency}`);
  };
  const showNotice = (n: MindNotice) => {
    if (n.level === 'ok' && n.source === 'health') return;
    for (const el of [...stack.children] as HTMLElement[]) {
      if (el.dataset.mid === n.id) remove(el);
    }
    const wrap = add(noticeCard(n, { compact: true, after: () => remove(wrap) }), n.level === 'danger' ? 0 : MIND_MS);
    wrap.dataset.mid = n.id;
    wrap.classList.add('mind', `lvl-${n.level}`);
  };

  store.on('notify', () => {
    if (store.notifyAdded) showNotification(store.notifyAdded);
    if (store.notifyClosed !== undefined) {
      for (const el of [...stack.children] as HTMLElement[]) {
        if (el.dataset.nid === String(store.notifyClosed)) remove(el);
      }
    }
  });
  store.on('mindNotices', () => {
    if (store.noticeAdded) showNotice(store.noticeAdded);
    const ids = new Set(store.notices().map((n) => n.id));
    for (const el of [...stack.children] as HTMLElement[]) {
      if (el.dataset.mid && !ids.has(el.dataset.mid)) remove(el);
    }
  });
  fit();
}
