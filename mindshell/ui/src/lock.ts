// The screensaver and the lock screen (kind=lock). The compositor decides
// when these windows exist and what they should be showing; this page draws
// it. One window per display: the screensaver runs on all of them, and while
// the session is locked the primary one also carries the card with the
// password field.
//
// The stages come from the compositor's `idle` event, relayed by the host as
// `lock`:
//
//   active      · locked: the card. Not locked: the window is on its way out.
//   screensaver · the saver, full screen, on every display.
//   blank       · the displays are off; draw nothing and stop the loop.

import * as bridge from './bridge';
import { h } from './dom';
import { icon } from './icons';
import { showKeyboard } from './controller';
import { play, type Session } from './gaming';
import { BLANK, startSaver } from './savers';

interface LockState {
  stage?: string;
  locked?: boolean;
  inhibited?: boolean;
  saver?: string;
}

interface LockInfo extends LockState {
  name?: string;
  display?: string;
  avatar?: string | null;
  host?: string;
  idle?: LockState;
}

export function renderLock(root: HTMLElement, arg: unknown): void {
  const a = (arg && typeof arg === 'object' ? arg : {}) as { primary?: boolean };
  const primary = a.primary !== false;

  root.classList.add('lock-window');
  const canvas = h('canvas', { class: 'lock-saver' }) as HTMLCanvasElement;
  const clockEl = clock();
  root.append(canvas, h('div', { class: 'g-vignette' }), clockEl);

  // ---- the saver ---------------------------------------------------------
  let running: string | null = null;
  let stopSaver: (() => void) | undefined;
  const saver = (id: string | null): void => {
    if (running === id) return;
    stopSaver?.();
    stopSaver = undefined;
    running = id;
    canvas.hidden = id === null;
    if (id) stopSaver = startSaver(canvas, id);
  };

  // ---- the card ----------------------------------------------------------
  const card = primary ? lockCard() : null;
  if (card) root.appendChild(card.el);

  let state: LockState = {};
  const apply = (next: LockState): void => {
    state = next;
    const stage = next.stage ?? 'active';
    const locked = !!next.locked;
    root.dataset.stage = stage;
    root.classList.toggle('locked', locked);
    // Locked and awake: the card, over a dimmed saver. Idling: the saver on
    // its own. Displays off: nothing, and the loop stops.
    const showCard = locked && stage === 'active';
    root.classList.toggle('carded', showCard);
    const chosen = next.saver ?? BLANK;
    saver(stage === 'blank' || chosen === BLANK ? null : chosen);
    clockEl.classList.toggle('drifting', !showCard);
    if (card) card.show(showCard);
  };

  bridge.on<LockState>('lock', (payload) => apply(payload ?? {}));
  void bridge
    .call<LockInfo>(primary ? 'lock.info' : 'lock.state')
    .then((info) => {
      if (card && primary) card.setUser(info.display || info.name || '', info.name ?? '', info.avatar ?? null, info.host ?? '');
      apply(info.idle ?? { stage: info.stage, locked: info.locked, saver: info.saver });
    })
    .catch(() => apply(state));

  // Anything at all here means somebody is at the keyboard; the compositor
  // has already woken the screen, and the password field wants the keys.
  root.addEventListener('mousedown', (e) => {
    const t = e.target as HTMLElement;
    if (!t.closest('button, input')) {
      e.preventDefault();
      card?.focus();
    }
  });
  window.addEventListener('keydown', () => card?.focus());
}

interface Card {
  el: HTMLElement;
  show(on: boolean): void;
  focus(): void;
  setUser(display: string, name: string, avatar: string | null, host: string): void;
}

function lockCard(): Card {
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
  const go = h('button', { type: 'submit', class: 'g-go', title: 'Unlock', 'aria-label': 'Unlock' }, icon('arrow-right', 18));
  const msg = h('div', { class: 'g-msg' });
  const caps = h('div', { class: 'g-caps', hidden: true }, icon('keyboard', 14), 'Caps Lock is on');
  const form = h('form', { class: 'g-form', onsubmit: (e: Event) => { e.preventDefault(); void submit(); } }, h('div', { class: 'g-field' }, input, go));
  const el = h('div', { class: 'lock-stage' }, h('div', { class: 'g-card lock-card' }, h('div', { class: 'lock-badge' }, icon('lock', 15), 'Locked'), avatar, who, whoSub, form, msg, caps));
  const cardEl = el.firstElementChild as HTMLElement;
  cardEl.append(h('button', { class: 'btn', type: 'button', onclick: () => showKeyboard(input) }, 'On-screen keyboard'));
  const resume = h('select', { 'aria-label': 'After unlocking' }, h('option', { value: '' }, 'Return to desktop'));
  cardEl.append(resume);
  void play<Session[]>('sessions').then(sessions => {
    for (const s of sessions.filter(s => s.active && s.suspended)) resume.append(h('option', { value: s.game }, `Resume ${s.game}`));
  }).catch(() => { resume.hidden = true; });
  let busy = false;

  const showMsg = (text: string, kind: 'error' | 'info' | '' = ''): void => {
    msg.textContent = text;
    msg.className = `g-msg${kind ? ` ${kind}` : ''}`;
  };

  const shake = (): void => {
    cardEl.classList.remove('shake');
    void cardEl.offsetWidth; // restart the animation
    cardEl.classList.add('shake');
  };

  async function submit(): Promise<void> {
    if (busy) return;
    const password = input.value;
    if (!password) {
      shake();
      return;
    }
    busy = true;
    cardEl.classList.add('busy');
    input.disabled = true;
    go.disabled = true;
    showMsg('');
    try {
      const r = await bridge.call<{ ok?: boolean; error?: string }>('lock.unlock', { password, resume: resume.value });
      if (r?.ok) {
        showMsg('Welcome back.', 'info');
        cardEl.classList.add('done');
      } else {
        input.value = '';
        shake();
        showMsg(r?.error || 'That password did not work.', 'error');
      }
    } catch (e) {
      input.value = '';
      shake();
      showMsg(String(e).replace(/^Error:\s*/, ''), 'error');
    } finally {
      busy = false;
      cardEl.classList.remove('busy');
      input.disabled = false;
      go.disabled = false;
      input.focus();
    }
  }

  const capsCheck = (e: KeyboardEvent): void => {
    caps.hidden = !e.getModifierState?.('CapsLock');
  };
  input.addEventListener('keydown', capsCheck);
  input.addEventListener('keyup', capsCheck);
  input.addEventListener('keydown', (e) => {
    if (e.key === 'Escape') {
      input.value = '';
      showMsg('');
    }
  });

  return {
    el,
    show(on) {
      el.hidden = !on;
      if (on) {
        cardEl.classList.remove('done');
        window.setTimeout(() => input.focus(), 30);
      } else {
        input.value = '';
        showMsg('');
      }
    },
    focus() {
      if (!el.hidden && !busy) input.focus();
    },
    setUser(display, name, avatarUrl, host) {
      who.textContent = display || name || 'Locked';
      whoSub.textContent = host ? `${name}@${host}` : name;
      whoSub.hidden = !whoSub.textContent;
      avatar.replaceChildren();
      if (avatarUrl) avatar.appendChild(h('img', { src: avatarUrl, alt: '' }));
      else {
        const words = (display || name || '?').trim().split(/\s+/).filter(Boolean);
        avatar.textContent = (words.length >= 2 ? words[0][0] + words[words.length - 1][0] : (words[0] ?? '?').slice(0, 2)).toUpperCase();
      }
    },
  };
}

/** The clock every lock window shows. While the saver is up it drifts slowly
 *  around the screen, so a panel never has the same pixels lit for hours. */
function clock(): HTMLElement {
  const time = h('span', { class: 'gc-time' });
  const ampm = h('span', { class: 'gc-ampm' });
  const date = h('div', { class: 'gc-date' });
  const el = h('div', { class: 'lock-clock' }, h('div', { class: 'gc-row' }, time, ampm), date);
  let driftMinute = -1;
  const tick = (): void => {
    const d = new Date();
    const hh = d.getHours();
    const values = [
      `${hh % 12 || 12}:${String(d.getMinutes()).padStart(2, '0')}`,
      hh >= 12 ? 'PM' : 'AM',
      d.toLocaleDateString(undefined, { weekday: 'long', month: 'long', day: 'numeric' }),
    ];
    [time, ampm, date].forEach((node, i) => {
      if (node.textContent !== values[i]) node.textContent = values[i];
    });
    if (el.classList.contains('drifting')) {
      const minute = Math.floor(d.getTime() / 60000);
      if (minute !== driftMinute) {
        driftMinute = minute;
        // Shift periodically to avoid burn-in, without a continuous CSS
        // transition that would force full-refresh compositing while idle.
        el.style.transform = `translate(${(Math.cos(minute / 3) * 11).toFixed(1)}vw, ${(Math.sin(minute / 4) * 5).toFixed(1)}vh)`;
      }
    } else {
      if (el.style.transform) el.style.transform = '';
      driftMinute = -1;
    }
  };
  tick();
  window.setInterval(tick, 5000);
  return el;
}
