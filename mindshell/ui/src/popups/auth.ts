// The authentication dialog (polkit). The host opens this popup when
// something asks polkit for authorisation — pkexec, a system app, the Mind
// running a privileged command — and closes it when the agent is done.
// Escape or a click beside it cancels the request (the host does that).

import * as bridge from '../bridge';
import { h } from '../dom';
import { icon } from '../icons';
import type { PolkitRequest } from '../types';
import type { PopupContent, PopupCtx } from './shared';

export function authPopup(ctx: PopupCtx): PopupContent {
  const store = ctx.store;
  const message = h('div', { class: 'auth-message' });
  const detail = h('div', { class: 'auth-detail mono' });
  const who = h('div', { class: 'auth-who' });
  const input = h('input', {
    type: 'password',
    class: 'auth-pw',
    placeholder: 'Password',
    autocomplete: 'off',
    autocapitalize: 'off',
    spellcheck: false,
    'aria-label': 'Password',
  }) as HTMLInputElement;
  const error = h('div', { class: 'auth-error', hidden: true });
  const cancel = h('button', { type: 'button', class: 'btn' }, 'Cancel');
  const go = h('button', { type: 'submit', class: 'btn accent' }, 'Authenticate');

  const req = () => store.state.polkit ?? undefined;
  const submit = () => {
    const r = req();
    if (!r || r.busy || !input.value) return;
    bridge.send('polkit.respond', { id: r.id, password: input.value });
    input.value = '';
    setBusy(true);
  };
  const setBusy = (busy: boolean) => {
    input.disabled = busy;
    go.disabled = busy;
    go.textContent = busy ? 'Checking…' : 'Authenticate';
  };
  cancel.addEventListener('click', () => {
    const r = req();
    if (r) bridge.send('polkit.cancel', { id: r.id });
    ctx.close();
  });

  const form = h(
    'form',
    { class: 'auth-form', onsubmit: (e: Event) => { e.preventDefault(); submit(); } },
    h('div', { class: 'auth-field' }, icon('lock', 15), input),
    error,
    h('div', { class: 'auth-actions' }, cancel, go),
  );
  const el = h(
    'div',
    { class: 'pop-body auth' },
    h('div', { class: 'auth-head' }, h('span', { class: 'auth-ic' }, icon('lock', 20)), h('div', {}, h('div', { class: 'pop-title' }, 'AUTHENTICATION REQUIRED'), who)),
    message,
    detail,
    form,
  );

  const render = () => {
    const r: PolkitRequest | undefined = req();
    if (!r) return;
    message.textContent = r.message || 'An application is asking for permission to make a system change.';
    // The command when polkit knows it (pkexec), otherwise the bare action id
    // — the detail line is there to say exactly what is being authorised.
    detail.textContent = r.command || r.action || '';
    detail.title = r.action ?? '';
    who.textContent = `as ${r.user}`;
    error.hidden = !r.error;
    const left = r.tries - r.attempt + 1;
    error.textContent = r.error ? (r.attempt > 1 ? `${r.error} (${left === 1 ? '1 try' : `${left} tries`} left)` : r.error) : '';
    setBusy(r.busy);
    if (!r.busy) {
      input.value = '';
      requestAnimationFrame(() => input.focus());
    }
  };
  render();
  store.bind(el, 'polkit', render);

  return { el, w: 400, focus: () => input.focus() };
}
