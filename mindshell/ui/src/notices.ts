// Mind notices and health findings as the shell shows them: level colours,
// action buttons and the confirm step for the risky ones (rollback, reboot).

import * as bridge from './bridge';
import { h } from './dom';
import { icon } from './icons';
import type { Finding, MindNotice, NoticeAction, NoticeLevel } from './types';

export function levelIcon(level: NoticeLevel | string): string {
  switch (level) {
    case 'danger':
      return 'warning';
    case 'warn':
      return 'warning';
    case 'ok':
      return 'check-circle';
    default:
      return 'info';
  }
}

export function sourceIcon(source: string): string {
  switch (source) {
    case 'updates':
      return 'package';
    case 'health':
      return 'pulse';
    default:
      return 'mind';
  }
}

/** Actions that change the system get a second click before they run. */
export function needsConfirm(a: NoticeAction): boolean {
  if (a.kind !== 'request') return false;
  const t = (a.arg as { type?: string } | null)?.type ?? '';
  return t === 'rollback' || t === 'power' || t === 'apply_updates';
}

export function runAction(a: NoticeAction): Promise<unknown> {
  return bridge.call('mind.act', { action: a });
}

export function actionButton(a: NoticeAction, after?: () => void, cls = 'btn small'): HTMLElement {
  const label = h('span', {}, a.label);
  const kindCls = a.kind === 'chat' ? ' mind' : needsConfirm(a) ? ' danger' : '';
  const btn = h('button', { class: cls + kindCls, title: a.kind === 'chat' ? 'Ask the Mind' : '' }, a.kind === 'chat' ? icon('mind', 13) : null, label);
  let armed = false;
  let t: ReturnType<typeof setTimeout> | undefined;
  btn.addEventListener('click', (e) => {
    e.stopPropagation();
    if (needsConfirm(a) && !armed) {
      armed = true;
      btn.classList.add('armed');
      label.textContent = `Confirm: ${a.label}`;
      t = setTimeout(() => {
        armed = false;
        btn.classList.remove('armed');
        label.textContent = a.label;
      }, 4000);
      return;
    }
    if (t) clearTimeout(t);
    btn.classList.add('busy');
    runAction(a)
      .catch((err) => console.warn('notice action failed', err))
      .finally(() => {
        btn.classList.remove('busy');
        after?.();
      });
  });
  return btn;
}

export interface NoticeCardOpts {
  /** A dismiss button (the notification centre). */
  dismiss?: (id: string) => void;
  /** Called after an action ran (a toast closes itself). */
  after?: () => void;
  compact?: boolean;
}

/** One notice (or health finding) as a card: icon, title, body, actions. */
export function noticeCard(n: MindNotice | Finding, opts: NoticeCardOpts = {}): HTMLElement {
  const source = 'source' in n ? n.source : 'health';
  const actions = h('div', { class: 'ntc-actions' });
  for (const a of n.actions ?? []) actions.appendChild(actionButton(a, opts.after));
  const close = opts.dismiss ? h('button', { class: 'ntc-x', title: 'Dismiss', onclick: (e: Event) => { e.stopPropagation(); opts.dismiss!(n.id); } }, icon('x', 13)) : null;
  return h(
    'div',
    { class: `ntc ntc-${n.level}${opts.compact ? ' compact' : ''}`, dataset: { id: n.id, source } },
    h('span', { class: 'ntc-ic' }, icon(sourceIcon(source), 16)),
    h('div', { class: 'ntc-text' }, h('div', { class: 'ntc-title' }, n.title), n.body ? h('div', { class: 'ntc-body' }, n.body) : null, n.actions?.length ? actions : null),
    close,
  );
}
