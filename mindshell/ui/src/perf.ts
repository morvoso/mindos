// Performance modes (mindos-perf): what each mode is, how to read the
// current one and how to switch. Shared by the panel widget, its popup and
// Settings › Performance.

import * as bridge from './bridge';
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

export function perfSubscribe(el: Element, cb: (s: PerfStatus | undefined) => void): void {
  if (!watching) {
    // Another window switched the mode (or GameMode did): read it again.
    watching = true;
    bridge.on('perf_changed', () => void perfRefresh(true));
  }
  const wrapped = (s: PerfStatus | undefined) => {
    if (!el.isConnected) return perfCache.listeners.delete(wrapped);
    cb(s);
  };
  perfCache.listeners.add(wrapped);
  if (perfCache.status) cb(perfCache.status);
}

export async function perfRefresh(force = false): Promise<PerfStatus | undefined> {
  if (!force && perfCache.status && Date.now() - perfCache.at < 3000) return perfCache.status;
  try {
    perfCache.status = await perfStatus();
  } catch (e) {
    console.warn('mindos-perf status failed', e);
    perfCache.status = undefined;
  }
  perfCache.at = Date.now();
  for (const cb of [...perfCache.listeners]) cb(perfCache.status);
  return perfCache.status;
}

/** Switch modes: optimistic locally, confirmed by the next status read. */
export async function perfSwitch(mode: PerfMode): Promise<string> {
  if (perfCache.status) {
    perfCache.status = { ...perfCache.status, mode };
    for (const cb of [...perfCache.listeners]) cb(perfCache.status);
  }
  const msg = await setPerfMode(mode);
  await perfRefresh(true);
  return msg;
}
