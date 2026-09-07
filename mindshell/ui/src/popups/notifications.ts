// The notification centre: what the Mind wants you to know, then the
// applications' notifications, newest first. Do not disturb and Clear all
// live in the header.

import * as bridge from '../bridge';
import { h } from '../dom';
import { icon } from '../icons';
import { noticeCard } from '../notices';
import type { Notification } from '../types';
import { openApp } from '../apps/shared';
import type { PopupContent, PopupCtx } from './shared';

export function timeAgo(secs: number): string {
  if (!secs) return '';
  const d = Math.max(0, Math.floor(Date.now() / 1000 - secs));
  if (d < 60) return 'now';
  if (d < 3600) return `${Math.floor(d / 60)} min`;
  if (d < 86400) return `${Math.floor(d / 3600)} h`;
  return `${Math.floor(d / 86400)} d`;
}

export function appIconFor(n: Notification, size = 28): HTMLElement {
  const glyph = () => h('span', { class: 'ntf-ic glyph' }, icon(n.urgency >= 2 ? 'warning' : 'bell', Math.round(size * 0.6)));
  if (!n.icon) return glyph();
  // An icon the theme does not have (a game's own name, say) becomes the glyph.
  const wrap = h('span', { class: 'ntf-ic-wrap' });
  const img = h('img', { class: 'ntf-ic', src: n.icon, width: size, height: size, alt: '' }) as HTMLImageElement;
  img.onerror = () => wrap.replaceChildren(glyph());
  wrap.appendChild(img);
  return wrap;
}

export function notificationCard(n: Notification, opts: { close?: (id: number) => void; after?: () => void; compact?: boolean } = {}): HTMLElement {
  const actions = h('div', { class: 'ntf-actions' });
  const dflt = n.actions.find((a) => a.key === 'default');
  for (const a of n.actions) {
    if (a.key === 'default') continue;
    actions.appendChild(
      h('button', { class: 'btn small', onclick: (e: Event) => { e.stopPropagation(); bridge.send('notify.action', { id: n.id, key: a.key }); opts.after?.(); } }, a.label),
    );
  }
  const close = opts.close ? h('button', { class: 'ntc-x', title: 'Dismiss', onclick: (e: Event) => { e.stopPropagation(); opts.close!(n.id); } }, icon('x', 13)) : null;
  const card = h(
    'div',
    { class: `ntf u${n.urgency}${opts.compact ? ' compact' : ''}${dflt ? ' clickable' : ''}`, dataset: { id: String(n.id) } },
    appIconFor(n),
    h(
      'div',
      { class: 'ntf-text' },
      h('div', { class: 'ntf-head' }, h('span', { class: 'ntf-app' }, n.app || 'Notification'), h('span', { class: 'ntf-time mono' }, timeAgo(n.time))),
      h('div', { class: 'ntf-summary' }, n.summary),
      n.body ? h('div', { class: 'ntf-body' }, n.body) : null,
      n.actions.some((a) => a.key !== 'default') ? actions : null,
    ),
    close,
  );
  if (dflt) {
    card.addEventListener('click', () => {
      bridge.send('notify.action', { id: n.id, key: 'default' });
      opts.after?.();
    });
  }
  return card;
}

export function notificationsPopup(ctx: PopupCtx): PopupContent {
  const store = ctx.store;
  const dndBtn = h('button', { class: 'btn small icon-btn', title: 'Do not disturb' }, icon('bell-off', 14));
  const clearBtn = h('button', { class: 'btn small', title: 'Clear every notification' }, 'Clear');
  const mindList = h('div', { class: 'ntc-list' });
  const appList = h('div', { class: 'ntf-list' });
  const mindHead = h('div', { class: 'pop-sub' }, h('span', { class: 'pop-title' }, 'MIND'), h('span', { class: 'strip-gap' }), h('button', { class: 'linkish', onclick: () => bridge.send('mind.open', { text: 'What should I know about my system right now?', ask: true }) }, icon('mind', 12), 'Ask'));
  const appHead = h('div', { class: 'pop-sub' }, h('span', { class: 'pop-title' }, 'APPS'), h('span', { class: 'strip-gap' }), clearBtn);
  const empty = h('div', { class: 'ntf-empty' }, icon('check-circle', 28), h('div', {}, 'All clear'), h('div', { class: 'pop-hint' }, 'Nothing needs your attention.'));
  const dndHint = h('div', { class: 'pop-hint dnd-hint' }, 'Do not disturb: only critical notifications pop up.');

  const render = () => {
    const dnd = !!store.state.notify?.dnd;
    dndBtn.classList.toggle('on', dnd);
    dndHint.hidden = !dnd;
    const notices = store.notices();
    mindList.replaceChildren(...notices.map((n) => noticeCard(n, { dismiss: (id) => bridge.send('mind.dismiss', { id }), after: () => ctx.relayout() })));
    mindHead.hidden = mindList.hidden = notices.length === 0;
    const items = [...store.notifications()].reverse();
    appList.replaceChildren(...items.map((n) => notificationCard(n, { close: (id) => bridge.send('notify.close', { id }), after: () => ctx.relayout() })));
    appHead.hidden = appList.hidden = items.length === 0;
    empty.hidden = notices.length + items.length > 0;
    ctx.relayout();
  };
  dndBtn.addEventListener('click', () => bridge.send('notify.setDnd', { enabled: !store.state.notify?.dnd }));
  clearBtn.addEventListener('click', () => bridge.send('notify.clear'));
  store.bind(mindList, 'notify', render);
  store.bind(mindList, 'mindNotices', render);
  store.bind(mindList, 'mind', render);

  const head = h('div', { class: 'pop-head' }, h('span', { class: 'pop-title' }, 'NOTIFICATIONS'), h('span', { class: 'strip-gap' }), dndBtn, h('button', { class: 'btn small icon-btn', title: 'Settings › Updates', onclick: () => { openApp('settings', 'updates'); ctx.close(); } }, icon('gear', 14)));
  const scroll = h('div', { class: 'pop-scroll' }, dndHint, mindHead, mindList, appHead, appList, empty);
  const el = h('div', { class: 'pop-body notifications' }, head, scroll);
  render();
  return { el, w: 400 };
}
