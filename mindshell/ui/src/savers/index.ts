// The screensavers. Each one is a small game that plays itself, drawn in the
// shell's own colours. They are all originals — the ideas are as old as home
// computers, the code and the artwork are ours.

import { breaker } from './breaker';
import { drift } from './drift';
import { lander } from './lander';
import { serpent } from './serpent';
import { starfield } from './starfield';
import { volley } from './volley';
import { wave } from './wave';
import type { Saver } from './types';

export type { Saver } from './types';

export const SAVERS: Saver[] = [serpent, volley, breaker, drift, wave, lander, starfield];

/** Pick a different one every few minutes. */
export const SHUFFLE = 'shuffle';
/** No screensaver: the screen simply goes black. */
export const BLANK = 'blank';

/** How long one saver runs before the shuffle moves on. */
const SHUFFLE_SECONDS = 240;

export function saverById(id: string | undefined | null): Saver | undefined {
  return SAVERS.find((s) => s.id === id);
}

/** What the Settings page lists, in order. */
export function saverOptions(): { value: string; label: string; description: string }[] {
  return [
    { value: SHUFFLE, label: 'Shuffle', description: 'A different one every few minutes.' },
    ...SAVERS.map((s) => ({ value: s.id, label: s.name, description: s.description })),
    { value: BLANK, label: 'Blank screen', description: 'Nothing at all: the screen just goes black.' },
  ];
}

/**
 * Start the saver `id` on `canvas` and return the function that stops it.
 * `shuffle` runs one after another; `blank` draws nothing.
 */
export function startSaver(canvas: HTMLCanvasElement, id: string): () => void {
  if (id === BLANK) return () => undefined;
  if (id !== SHUFFLE) {
    const saver = saverById(id);
    return saver ? saver.start(canvas) : startSaver(canvas, SHUFFLE);
  }

  let stopCurrent: (() => void) | undefined;
  let timer: ReturnType<typeof setTimeout> | undefined;
  let last = '';
  const next = (): void => {
    stopCurrent?.();
    const choices = SAVERS.filter((s) => s.id !== last);
    const saver = choices[Math.floor(Math.random() * choices.length)] ?? SAVERS[0];
    last = saver.id;
    stopCurrent = saver.start(canvas);
    timer = setTimeout(next, SHUFFLE_SECONDS * 1000);
  };
  next();
  return () => {
    if (timer !== undefined) clearTimeout(timer);
    stopCurrent?.();
  };
}
