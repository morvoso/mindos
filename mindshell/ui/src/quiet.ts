// Quiet while a game runs. GameMode tells mindos-perf, mindos-perf tells the
// host (`/run/mindos/perf/game`) and the host tells every page: the frames
// belong to the game, so the shell drops its animations, and its samplers tick
// far slower — the desktop stops sampling altogether, since the game covers it.

/** How much longer a poll waits while a game runs. */
const SLOWER = 5;

let quiet = false;
const listeners = new Set<(quiet: boolean) => void>();

export function isQuiet(): boolean {
  return quiet;
}

/** `:root.quiet` in the stylesheet; the host's `game` state sets it. */
export function setQuiet(value: boolean): void {
  if (value === quiet) return;
  quiet = value;
  document.documentElement.classList.toggle('quiet', quiet);
  for (const cb of [...listeners]) cb(quiet);
}

export function onQuiet(cb: (quiet: boolean) => void): () => void {
  listeners.add(cb);
  return () => listeners.delete(cb);
}

/** What a `ms` poll should wait now; 0 while it is not worth polling at all. */
export function quietInterval(ms: number): number {
  if (!quiet) return ms;
  return document.documentElement.dataset.kind === 'desktop' ? 0 : ms * SLOWER;
}
