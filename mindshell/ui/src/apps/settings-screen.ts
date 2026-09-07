// Settings › Screen: how long before the screensaver comes up, before the
// session locks and before the displays switch off, which screensaver runs,
// and the two rules that tie them together (lock with the displays, lock
// when the machine sleeps).
//
// The timings live in the compositor's preferences, so they take effect the
// moment they are saved and survive a restart.

import * as bridge from '../bridge';
import { h } from '../dom';
import { icon } from '../icons';
import { SAVERS, saverOptions, startSaver } from '../savers';
import { store } from '../state';
import type { IdlePrefs } from '../types';
import { card, notice, pageHeader, row, selectBox, toggle } from './shared';

const DEFAULTS: IdlePrefs = {
  screensaver: 300,
  saver: 'shuffle',
  lock: 900,
  blank: 900,
  lock_on_blank: true,
  lock_on_sleep: true,
  stay_awake_when_busy: true,
};

/** The delays every menu offers, in seconds. */
const DELAYS = [60, 120, 300, 600, 900, 1800, 2700, 3600, 7200];

function delayOptions(): { value: number; label: string }[] {
  return DELAYS.map((s) => ({ value: s, label: s < 3600 ? `${s / 60} minutes` : s === 3600 ? '1 hour' : `${s / 3600} hours` }));
}

export function screenPage(el: HTMLElement, root: HTMLElement): () => void {
  const note = notice();
  let idle: IdlePrefs = { ...DEFAULTS, ...(store.prefs.idle ?? {}) };
  const stops: (() => void)[] = [];

  /** Save the lot; the compositor merges and echoes the result back. */
  const save = (patch: Partial<IdlePrefs>): void => {
    idle = { ...idle, ...patch };
    void store.setPrefs({ idle: { ...idle } });
    sync();
  };

  // ---- the screensaver -----------------------------------------------------
  const saverOn = toggle(idle.screensaver > 0, (v) => save({ screensaver: v ? (idle.screensaver || 300) : 0 }));
  const saverAfter = selectBox(delayOptions(), idle.screensaver || 300, (v) => save({ screensaver: v }));
  const tiles = h('div', { class: 'saver-grid' });
  const chosen = h('div', { class: 'saver-chosen' });

  const setSaver = (id: string): void => {
    save({ saver: id });
  };

  saverOptions().forEach((option, index) => {
    const one = SAVERS.find((s) => s.id === option.value);
    const preview = h('div', { class: 'saver-preview', dataset: { empty: one ? '' : option.value } });
    const tile = h(
      'button',
      { class: 'saver-tile', type: 'button', onclick: () => setSaver(option.value), dataset: { saver: option.value }, title: option.description },
      preview,
      h('div', { class: 'saver-name' }, option.label),
    );
    tiles.appendChild(tile);
    if (!one) return;

    // A canvas keeps whatever was drawn on it last, so each tile runs for a
    // moment when the page opens and freezes on that picture. Pointing at a
    // tile starts it again; a grid of games all running at once is more work
    // than a settings page should be doing.
    const canvas = h('canvas', { class: 'saver-canvas' }) as HTMLCanvasElement;
    const frozen = h('img', { class: 'saver-frozen', alt: '', hidden: true }) as HTMLImageElement;
    preview.append(canvas, frozen);
    let stop: (() => void) | undefined;
    const rest = (): void => {
      if (stop) {
        // Resizing a canvas wipes it, so keep the last frame as a picture.
        try {
          frozen.src = canvas.toDataURL();
          frozen.hidden = false;
        } catch {
          /* nothing to freeze */
        }
      }
      stop?.();
      stop = undefined;
    };
    const play = (): void => {
      frozen.hidden = true;
      if (!stop && canvas.isConnected) stop = one.start(canvas);
    };
    stops.push(rest);
    const warm = window.setTimeout(() => {
      play();
      window.setTimeout(rest, 2600);
    }, 120 + index * 220);
    stops.push(() => window.clearTimeout(warm));
    tile.addEventListener('pointerenter', play);
    tile.addEventListener('pointerleave', rest);
    tile.addEventListener('focus', play);
    tile.addEventListener('blur', rest);
  });

  const previewBtn = h('button', { class: 'btn', onclick: () => fullPreview(root, idle.saver) }, icon('eye', 15), 'Preview full screen');

  // ---- the lock ------------------------------------------------------------
  const lockOn = toggle(idle.lock > 0, (v) => save({ lock: v ? (idle.lock || 900) : 0 }));
  const lockAfter = selectBox(delayOptions(), idle.lock || 900, (v) => save({ lock: v }));
  const lockOnBlank = toggle(idle.lock_on_blank, (v) => save({ lock_on_blank: v }));
  const lockOnSleep = toggle(idle.lock_on_sleep, (v) => save({ lock_on_sleep: v }));
  const lockNow = h('button', { class: 'btn', onclick: () => { bridge.send('lock.now'); note.show('The screen is locked.', 'ok'); } }, icon('lock', 15), 'Lock now');

  // ---- the displays --------------------------------------------------------
  const blankOn = toggle(idle.blank > 0, (v) => save({ blank: v ? (idle.blank || 900) : 0 }));
  const blankAfter = selectBox(delayOptions(), idle.blank || 900, (v) => save({ blank: v }));
  const awake = toggle(idle.stay_awake_when_busy, (v) => save({ stay_awake_when_busy: v }));
  const blankNow = h('button', { class: 'btn', onclick: () => { bridge.send('lock.blank'); note.show('The displays are off. Move the mouse to bring them back.', 'ok'); } }, icon('monitor', 15), 'Turn the displays off now');

  el.append(
    pageHeader('Screen', 'What happens when the machine is left alone.'),
    note.el,
    card(
      'Screensaver',
      row('Show a screensaver', 'A game that plays itself while nobody is using the machine.', saverOn),
      row('After', null, saverAfter),
      h('div', { class: 'card-help' }, 'Pick one, or let it shuffle. Move the pointer over a tile to watch it.'),
      tiles,
      h('div', { class: 'card-actions' }, previewBtn, chosen),
    ),
    card(
      'Lock',
      row('Lock the screen', 'Ask for the password before the desktop comes back.', lockOn),
      row('After', null, lockAfter),
      row('Lock when the displays turn off', 'The screen is locked at the same moment the displays go dark.', lockOnBlank),
      row('Lock when the computer sleeps', 'Locked before it suspends, so the desktop is never up when it wakes.', lockOnSleep),
      h('div', { class: 'card-actions' }, lockNow, h('span', { class: 'row-help' }, 'Super + L does the same thing.')),
    ),
    card(
      'Displays',
      row('Turn the displays off', 'The displays are switched off properly, not just painted black.', blankOn),
      row('After', null, blankAfter),
      row('Stay awake while something is playing', 'A video player or a game can hold all of this off while it runs.', awake),
      h('div', { class: 'card-actions' }, blankNow),
    ),
  );

  function sync(): void {
    const s = idle.screensaver > 0;
    saverAfter.disabled = !s;
    tiles.classList.toggle('off', !s);
    for (const tile of Array.from(tiles.children) as HTMLElement[]) tile.classList.toggle('on', tile.dataset.saver === idle.saver);
    const picked = saverOptions().find((o) => o.value === idle.saver);
    chosen.textContent = picked ? picked.description : '';
    lockAfter.disabled = idle.lock === 0;
    blankAfter.disabled = idle.blank === 0;
    (saverOn.querySelector('input') as HTMLInputElement).checked = idle.screensaver > 0;
    (lockOn.querySelector('input') as HTMLInputElement).checked = idle.lock > 0;
    (blankOn.querySelector('input') as HTMLInputElement).checked = idle.blank > 0;
    (lockOnBlank.querySelector('input') as HTMLInputElement).checked = idle.lock_on_blank;
    (lockOnSleep.querySelector('input') as HTMLInputElement).checked = idle.lock_on_sleep;
    (awake.querySelector('input') as HTMLInputElement).checked = idle.stay_awake_when_busy;
    if (idle.screensaver > 0) saverAfter.value = String(idle.screensaver);
    if (idle.lock > 0) lockAfter.value = String(idle.lock);
    if (idle.blank > 0) blankAfter.value = String(idle.blank);
  }

  store.bind(el, 'prefs', () => {
    idle = { ...DEFAULTS, ...(store.prefs.idle ?? {}) };
    sync();
  });
  void store.fetchPrefs();
  sync();

  return () => {
    for (const stop of stops) stop();
  };
}

/** Run the chosen saver over the whole app window until it is clicked. */
function fullPreview(root: HTMLElement, id: string): void {
  const canvas = h('canvas', { class: 'saver-canvas' }) as HTMLCanvasElement;
  const hint = h('div', { class: 'saver-hint' }, 'Click anywhere, or press Escape, to stop the preview');
  const layer = h('div', { class: 'saver-full' }, canvas, hint);
  const stop = startSaver(canvas, id);
  const close = (): void => {
    stop();
    layer.remove();
    window.removeEventListener('keydown', onKey, true);
  };
  const onKey = (e: KeyboardEvent): void => {
    e.preventDefault();
    e.stopPropagation();
    close();
  };
  layer.addEventListener('pointerdown', close);
  window.addEventListener('keydown', onKey, true);
  root.appendChild(layer);
}
