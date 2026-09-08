// The login screen: `mindshell --app greeter`, started by greetd through
// mindos-greeter. One window per output; the primary one carries the clock,
// the login card, the user chips and the power buttons, the others the
// wallpaper and the clock. greetd does the authenticating: the host relays
// the conversation (greeter.login / greeter.respond) and reports the outcome.

import * as bridge from './bridge';
import { appearanceControls } from './appearance';
import { h } from './dom';
import { icon } from './icons';
import { showKeyboard } from './controller';

interface GUser {
  name: string;
  display: string;
  avatar?: string | null;
}

interface GSession {
  id: string;
  name: string;
  exec: string;
}

interface GInfo {
  users: GUser[];
  sessions: GSession[];
  last: { user?: string; session?: string };
  host: string;
}

type Outcome =
  | { status: 'started' }
  | { status: 'prompt'; secret: boolean; message: string; notes: string[] }
  | { status: 'failed'; message: string; notes: string[] };

const ARM_MS = 4000;

export function renderGreeter(root: HTMLElement, arg: unknown): void {
  const a = (arg && typeof arg === 'object' ? arg : {}) as { primary?: boolean };
  const primary = a.primary !== false;
  root.classList.add('greeter-window');
  const wall = h('div', { class: 'wallpaper builtin' });
  root.append(wall, h('div', { class: 'g-vignette' }), clock());
  if (!primary) return;
  const appearance = appearanceControls();
  root.append(h('div', { class: 'g-appearance' }, h('strong', {}, 'MINDOS'), appearance.el));

  const stage = h('div', { class: 'g-stage' });
  const foot = h('footer', { class: 'g-foot' });
  root.append(stage, foot);

  void bridge
    .call<GInfo>('greeter.info')
    .then((info) => build(root, stage, foot, info))
    .catch((e) => {
      stage.appendChild(h('div', { class: 'g-card' }, h('div', { class: 'g-msg error' }, `The login screen cannot start: ${e}`)));
    });
}

function build(root: HTMLElement, stage: HTMLElement, foot: HTMLElement, info: GInfo): void {
  const users = info.users.length ? info.users : [{ name: '', display: 'Sign in', avatar: null }];
  const sessions = info.sessions.length ? info.sessions : [{ id: 'mindos', name: 'MindOS', exec: 'mindos-session' }];
  let current = users.find((u) => u.name === info.last.user) ?? users[0];
  let session = sessions.find((s) => s.id === info.last.session)?.id ?? sessions[0].id;
  let busy = false;
  let prompting = false;

  // ----- the card -------------------------------------------------------------
  const avatar = h('div', { class: 'g-avatar' });
  const who = h('div', { class: 'g-who' });
  const whoSub = h('div', { class: 'g-who-sub' });
  const input = h('input', {
    type: 'password',
    class: 'g-pw',
    placeholder: 'Password',
    autocomplete: 'off',
    autocapitalize: 'off',
    spellcheck: false,
    'aria-label': 'Password',
  }) as HTMLInputElement;
  const go = h('button', { type: 'submit', class: 'g-go', title: 'Sign in', 'aria-label': 'Sign in' }, icon('arrow-right', 18));
  const form = h('form', { class: 'g-form', onsubmit: (e: Event) => { e.preventDefault(); void submit(); } }, h('div', { class: 'g-field' }, input, go));
  const msg = h('div', { class: 'g-msg' });
  const caps = h('div', { class: 'g-caps', hidden: true }, icon('keyboard', 14), 'Caps Lock is on');
  const card = h('div', { class: 'g-card' }, avatar, who, whoSub, form, msg, caps);
  stage.appendChild(card);
  card.append(h('button', { class: 'btn', type: 'button', onclick: () => showKeyboard(input) }, 'On-screen keyboard · controller A'));

  // Other accounts, when there are any.
  const chips = new Map<string, HTMLElement>();
  const userRow = h('div', { class: 'g-users', hidden: users.length < 2 });
  for (const u of users) {
    const chip = h('button', { class: 'g-chip', type: 'button', onclick: () => select(u) }, avatarEl(u, 'g-chip-av'), h('span', {}, u.display));
    chips.set(u.name, chip);
    userRow.appendChild(chip);
  }
  stage.appendChild(userRow);

  // The session, when there is a choice.
  if (sessions.length > 1) {
    const sel = h('select', { class: 'g-session-sel', 'aria-label': 'Session' }) as HTMLSelectElement;
    for (const s of sessions) sel.appendChild(h('option', { value: s.id, selected: s.id === session }, s.name));
    sel.addEventListener('change', () => {
      session = sel.value;
    });
    stage.appendChild(h('label', { class: 'g-session' }, icon('layers', 14), sel, h('span', { class: 'g-session-caret' }, icon('chevron-down', 12))));
  }

  // ----- the footer ------------------------------------------------------------
  foot.append(
    h('div', { class: 'g-brand' }, h('span', { class: 'g-brand-text' }, 'MINDOS'), h('span', { class: 'g-brand-sub' }, info.host || 'gaming')),
    h('div', { class: 'g-power' }, powerButton('Restart', 'reboot', 'reboot'), powerButton('Power off', 'power', 'poweroff')),
  );

  // ----- behaviour -------------------------------------------------------------
  function select(u: GUser): void {
    if (prompting) cancelPrompt();
    current = u;
    avatar.replaceChildren(...avatarEl(u, '').childNodes);
    who.textContent = u.display;
    whoSub.textContent = u.name && u.name !== u.display ? u.name : '';
    whoSub.hidden = !whoSub.textContent;
    for (const [name, chip] of chips) chip.classList.toggle('on', name === u.name);
    input.value = '';
    showMsg('');
    input.focus();
  }

  function showMsg(text: string, kind: 'error' | 'info' | '' = ''): void {
    msg.textContent = text;
    msg.className = `g-msg${kind ? ` ${kind}` : ''}`;
  }

  function setBusy(v: boolean): void {
    busy = v;
    card.classList.toggle('busy', v);
    input.disabled = v;
    go.disabled = v;
  }

  function shake(): void {
    card.classList.remove('shake');
    void card.offsetWidth; // restart the animation
    card.classList.add('shake');
  }

  function resetPrompt(): void {
    prompting = false;
    input.type = 'password';
    input.placeholder = 'Password';
    input.setAttribute('aria-label', 'Password');
  }

  function cancelPrompt(): void {
    bridge.send('greeter.cancel');
    resetPrompt();
  }

  async function submit(): Promise<void> {
    if (busy) return;
    const value = input.value;
    if (!prompting && !value) {
      shake();
      input.focus();
      return;
    }
    setBusy(true);
    showMsg('');
    try {
      const outcome = prompting
        ? await bridge.call<Outcome>('greeter.respond', { response: value, session })
        : await bridge.call<Outcome>('greeter.login', { user: current.name, password: value, session });
      handle(outcome);
    } catch (e) {
      resetPrompt();
      input.value = '';
      shake();
      showMsg(String(e).replace(/^Error:\s*/, ''), 'error');
    } finally {
      setBusy(false);
      if (!root.classList.contains('leaving')) input.focus();
    }
  }

  function handle(outcome: Outcome): void {
    if (outcome.status === 'started') {
      welcome();
      return;
    }
    input.value = '';
    if (outcome.status === 'prompt') {
      prompting = true;
      input.type = outcome.secret ? 'password' : 'text';
      const label = outcome.message.replace(/:\s*$/, '') || 'Answer';
      input.placeholder = label;
      input.setAttribute('aria-label', label);
      showMsg(outcome.notes.join(' '), 'info');
      return;
    }
    resetPrompt();
    shake();
    const detail = outcome.notes.filter(Boolean).join(' ');
    showMsg(detail || 'That password did not work.', 'error');
  }

  function welcome(): void {
    showMsg(`Welcome, ${current.display}.`, 'info');
    card.classList.add('done');
    root.classList.add('leaving');
    window.setTimeout(() => bridge.send('greeter.done'), 450);
  }

  function powerButton(label: string, ic: string, action: string): HTMLElement {
    const text = h('span', {}, label);
    let armed: number | undefined;
    const disarm = () => {
      if (armed !== undefined) window.clearTimeout(armed);
      armed = undefined;
      b.classList.remove('armed');
      text.textContent = label;
    };
    const b = h('button', { class: 'g-pbtn', type: 'button', title: label }, icon(ic, 15), text);
    b.addEventListener('click', () => {
      if (armed !== undefined) {
        disarm();
        b.classList.add('busy');
        bridge.send('greeter.power', { action });
        return;
      }
      b.classList.add('armed');
      text.textContent = `Click again to ${label.toLowerCase()}`;
      armed = window.setTimeout(disarm, ARM_MS);
    });
    b.addEventListener('blur', disarm);
    return b;
  }

  const capsCheck = (e: KeyboardEvent) => {
    caps.hidden = !e.getModifierState?.('CapsLock');
  };
  input.addEventListener('keydown', capsCheck);
  input.addEventListener('keyup', capsCheck);
  root.addEventListener('keydown', (e) => {
    if (e.key === 'Escape') {
      if (prompting) cancelPrompt();
      input.value = '';
      showMsg('');
      input.focus();
    }
  });
  // The keyboard belongs to this window; keep the field focused.
  root.addEventListener('mousedown', (e) => {
    const t = e.target as HTMLElement;
    if (!t.closest('button, select, input')) {
      e.preventDefault();
      input.focus();
    }
  });

  select(current);
  window.setTimeout(() => input.focus(), 50);
}

function initials(u: GUser): string {
  const words = (u.display || u.name).trim().split(/\s+/).filter(Boolean);
  const s = words.length >= 2 ? words[0][0] + words[words.length - 1][0] : (words[0] ?? '?').slice(0, 2);
  return s.toUpperCase();
}

function avatarEl(u: GUser, cls: string): HTMLElement {
  const el = h('div', { class: cls });
  if (u.avatar) el.appendChild(h('img', { src: u.avatar, alt: '' }));
  else el.textContent = initials(u);
  return el;
}

function clock(): HTMLElement {
  const time = h('span', { class: 'gc-time' });
  const ampm = h('span', { class: 'gc-ampm' });
  const date = h('div', { class: 'gc-date' });
  const el = h('div', { class: 'greeter-clock' }, h('div', { class: 'gc-row' }, time, ampm), date);
  const tick = () => {
    const d = new Date();
    const hh = d.getHours();
    time.textContent = `${hh % 12 || 12}:${String(d.getMinutes()).padStart(2, '0')}`;
    ampm.textContent = hh >= 12 ? 'PM' : 'AM';
    date.textContent = d.toLocaleDateString(undefined, { weekday: 'long', month: 'long', day: 'numeric' });
  };
  tick();
  window.setInterval(tick, 5000);
  return el;
}
