// Performance modes (mindos-perf): what each mode is, how to read the
// current one and how to switch. Shared by the panel widget, its popup and
// Settings › Performance.

import * as bridge from './bridge';
import { every } from './dom';
import { isQuiet } from './quiet';
import type { PerfMode, PerfStatus, RunResult } from './types';

export interface PerfModeInfo {
  mode: PerfMode;
  label: string;
  icon: string;
  blurb: string;
  detail: string;
}

export const PERF_MODES: PerfModeInfo[] = [
  { mode: 'balanced', label: 'Balanced', icon: 'gauge', blurb: 'Full speed under load, low power when idle.', detail: 'schedutil governor, EPP balance, boost on, EEVDF + BORE scheduler, transparent huge pages, proactive compaction.' },
  { mode: 'performance', label: 'Performance', icon: 'rocket', blurb: 'Maximum clocks and lowest latency for games.', detail: 'Performance governor and EPP, boost on, the scx_lavd scheduler for games, no proactive compaction, swappiness 10, NVIDIA persistence mode.' },
  { mode: 'quiet', label: 'Quiet', icon: 'leaf', blurb: 'Reduced power draw and fan noise.', detail: 'Powersave governor, EPP power, boost off, the low-power platform profile, huge pages on request only.' },
];

export function modeInfo(mode: string | undefined): PerfModeInfo {
  return PERF_MODES.find((m) => m.mode === mode) ?? PERF_MODES[0];
}

export async function perfStatus(): Promise<PerfStatus | undefined> {
  const r = await bridge.call<RunResult>('shell.run', { argv: ['mindos-perf', 'status', '--json'] });
  if (!r.ok || !r.json) throw new Error(r.stderr.trim() || `mindos-perf exited ${r.status}`);
  return r.json as PerfStatus;
}

export async function setPerfMode(mode: PerfMode): Promise<string> {
  const r = await bridge.call<RunResult>('shell.run', { argv: ['sudo', '-n', 'mindos-perf', 'set', mode] });
  if (!r.ok) throw new Error(r.stderr.trim() || r.stdout.trim() || `mindos-perf exited ${r.status}`);
  return r.stdout.trim();
}

export async function setPerfConfig(key: string, value: string): Promise<void> {
  const r = await bridge.call<RunResult>('shell.run', { argv: ['sudo', '-n', 'mindos-perf', 'config', key, value] });
  if (!r.ok) throw new Error(r.stderr.trim() || r.stdout.trim() || `mindos-perf exited ${r.status}`);
}

/** Every panel widget and popup shares the last status so they agree at once. */
export const perfCache: { status?: PerfStatus; at: number; listeners: Set<(s: PerfStatus | undefined) => void> } = { at: 0, listeners: new Set() };

let watching = false;
let stale = false;

export function perfSubscribe(el: Element, cb: (s: PerfStatus | undefined) => void): () => void {
  if (!watching) {
    // Another window switched the mode (or GameMode did): read it again. The
    // host's broadcast is what keeps every page current; a hidden page waits
    // until it is shown, so a change costs one helper run per visible page.
    watching = true;
    bridge.on('perf_changed', () => {
      if (document.hidden) stale = true;
      else void perfRefresh(true);
    });
    document.addEventListener('visibilitychange', () => {
      if (document.hidden || !stale) return;
      stale = false;
      void perfRefresh(true);
    });
  }
  const wrapped = (s: PerfStatus | undefined) => {
    if (!el.isConnected) return perfCache.listeners.delete(wrapped);
    cb(s);
  };
  perfCache.listeners.add(wrapped);
  if (perfCache.status) cb(perfCache.status);
  return () => { perfCache.listeners.delete(wrapped); };
}

let pending: Promise<PerfStatus | undefined> | undefined;

export async function perfRefresh(force = false): Promise<PerfStatus | undefined> {
  if (pending) {
    if (!force) return pending;
    await pending;
    if (pending) return pending;
  }
  if (!force && perfCache.status && Date.now() - perfCache.at < 3000) return perfCache.status;
  pending = (async () => {
    try {
      perfCache.status = await perfStatus();
    } catch (e) {
      console.warn('mindos-perf status failed', e);
      perfCache.status = undefined;
    }
    perfCache.at = Date.now();
    for (const cb of [...perfCache.listeners]) cb(perfCache.status);
    return perfCache.status;
  })().finally(() => { pending = undefined; });
  return pending;
}

/** The safety net behind the `perf_changed` broadcast: a rare poll that is
 *  skipped while the page is hidden, while a game runs, or when the status was
 *  read recently anyway. The first call is immediate (it is the mount-time
 *  read; a refresh already in flight is shared, not repeated). */
export function perfWatch(el: Element, ms = 60_000): () => void {
  return every(el, ms, () => {
    if (document.hidden || isQuiet()) return;
    if (perfCache.status && Date.now() - perfCache.at < ms / 2) return;
    void perfRefresh();
  });
}

/** Show the confirmed state, including a saved preference while gaming. */
export async function perfSwitch(mode: PerfMode): Promise<string> {
  try {
    return await setPerfMode(mode);
  } finally {
    await perfRefresh(true);
  }
}
