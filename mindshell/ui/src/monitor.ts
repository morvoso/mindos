// The pieces every system reading is drawn with: one shared sampler, the
// graphs, and the units.
//
// The sampler matters more than it looks. A desktop can have the readout in
// the rail, a Task Manager widget on the wallpaper and the sysmon panel all
// asking about the same machine at the same time; each one asking for itself
// would walk `/proc` three times a second to draw the same number three ways.
// So callers declare what they need, the demands are unioned, and one call
// every interval answers all of them.

import * as bridge from './bridge';
import { anchored, every, h } from './dom';
import type { Overview } from './types';

/** What a subscriber needs from `system.overview`. */
export interface Demand {
  /** Sections to collect; everything, if not given. */
  parts?: OverviewPart[];
  /** How many of the busiest processes to include. */
  top?: number;
}

export type OverviewPart = 'storage' | 'net' | 'sensors' | 'host' | 'containers';

const demands = new Set<Demand>();
let inflight: Promise<Overview> | undefined;
let last: { at: number; value: Overview } | undefined;

/** Register a standing need, so the shared call collects enough for it. */
export function wantOverview(d: Demand): () => void {
  demands.add(d);
  return () => {
    demands.delete(d);
  };
}

function request(): Record<string, unknown> {
  let top = 0;
  const parts = new Set<string>();
  let all = demands.size === 0;
  for (const d of demands) {
    top = Math.max(top, d.top ?? 0);
    if (!d.parts) all = true;
    else for (const p of d.parts) parts.add(p);
  }
  return all ? { top } : { top, parts: [...parts] };
}

/**
 * The machine, as of at most `maxAgeMs` ago. A call already on its way is
 * awaited rather than joined by a second one: two pages refreshing on the same
 * beat cost one walk of `/proc`, not two.
 */
export function overview(maxAgeMs = 400): Promise<Overview> {
  if (last && Date.now() - last.at < maxAgeMs) return Promise.resolve(last.value);
  if (inflight) return inflight;
  inflight = bridge
    .call<Overview>('system.overview', request())
    .then((value) => {
      last = { at: Date.now(), value };
      return value;
    })
    .finally(() => {
      inflight = undefined;
    });
  return inflight;
}

/**
 * Sample the machine into `fn` for as long as `el` is in the document. Ticks
 * stop while the page is hidden and slow while a game is running (see
 * `every`), and are skipped outright while `el` is not being displayed: the
 * desktop rail stays mounted behind an open Settings page, and a reading
 * nobody can see is a reading not worth taking.
 *
 * The repaint happens under `anchored`, so a reading that changes the height
 * of something above the viewport does not shift what is being read.
 */
export function poll(el: HTMLElement, ms: number, demand: Demand, fn: (v: Overview) => void, onError?: (e: unknown) => void): () => void {
  const release = wantOverview(demand);
  let busy = false;
  const stop = every(el, ms, () => {
    if (busy || el.offsetParent === null) return;
    busy = true;
    overview(ms / 2)
      .then((v) => {
        if (el.isConnected) anchored(el, () => fn(v));
      })
      .catch((e) => {
        if (el.isConnected) onError?.(e);
      })
      .finally(() => {
        busy = false;
      });
  });
  return () => {
    stop();
    release();
  };
}

// ------------------------------------------------------------------- units

/** Bytes, in the shortest form that stays honest: 4 GB, 812 MB, 1.2 kB. */
export function bytes(n: number | null | undefined, digits = 1): string {
  if (n == null || !Number.isFinite(n)) return '—';
  const units = ['B', 'kB', 'MB', 'GB', 'TB', 'PB'];
  let v = Math.abs(n);
  let i = 0;
  while (v >= 1024 && i < units.length - 1) {
    v /= 1024;
    i += 1;
  }
  const shown = i === 0 ? Math.round(v) : v >= 100 ? Math.round(v) : Number(v.toFixed(v >= 10 ? Math.min(digits, 1) : digits));
  return `${n < 0 ? '-' : ''}${shown} ${units[i]}`;
}

export function rate(bytesPerSecond: number | null | undefined): string {
  if (bytesPerSecond == null || !Number.isFinite(bytesPerSecond)) return '—';
  if (bytesPerSecond < 1) return '0 B/s';
  return `${bytes(bytesPerSecond)}/s`;
}

/** MHz as the machine's own badge would print it. */
export function hertz(mhz: number | null | undefined): string {
  if (mhz == null || !Number.isFinite(mhz) || mhz <= 0) return '—';
  return mhz >= 1000 ? `${(mhz / 1000).toFixed(2)} GHz` : `${Math.round(mhz)} MHz`;
}

export function degrees(c: number | null | undefined): string {
  return c == null || !Number.isFinite(c) ? '—' : `${Math.round(c)}°`;
}

export function percent(v: number | null | undefined, digits = 0): string {
  return v == null || !Number.isFinite(v) ? '—' : `${v.toFixed(digits)}%`;
}

/** Big numbers without the noise: 14.2k context switches, not 14240. */
export function count(n: number | null | undefined): string {
  if (n == null || !Number.isFinite(n)) return '—';
  if (Math.abs(n) < 1000) return String(Math.round(n));
  if (Math.abs(n) < 1_000_000) return `${(n / 1000).toFixed(1)}k`;
  return `${(n / 1_000_000).toFixed(1)}M`;
}

export function duration(seconds: number): string {
  if (!Number.isFinite(seconds) || seconds < 0) return '—';
  const d = Math.floor(seconds / 86400);
  const hh = Math.floor((seconds % 86400) / 3600);
  const mm = Math.floor((seconds % 3600) / 60);
  const ss = Math.floor(seconds % 60);
  if (d > 0) return `${d}d ${hh}h ${mm}m`;
  if (hh > 0) return `${hh}h ${String(mm).padStart(2, '0')}m`;
  if (mm > 0) return `${mm}m ${String(ss).padStart(2, '0')}s`;
  return `${ss}s`;
}

/**
 * The colour a load wears. Calm below two thirds, warm approaching the limit,
 * alarming at it: the same three steps every meter in the shell uses, so a
 * glance at any of them means the same thing.
 */
export function heat(pct: number | null | undefined): string {
  if (pct == null || !Number.isFinite(pct)) return 'var(--fg-faint)';
  if (pct >= 90) return 'var(--danger)';
  if (pct >= 70) return 'var(--warn)';
  return 'var(--accent)';
}

/** The same three steps as a class, for anything CSS colours itself. */
export function heatClass(pct: number | null | undefined): string {
  if (pct == null || !Number.isFinite(pct)) return '';
  return pct >= 90 ? 'hot' : pct >= 70 ? 'warm' : '';
}

// ------------------------------------------------------------------ charts

export interface Chart {
  el: HTMLElement;
  /** Add one reading per series; the oldest scrolls off the left. */
  push(...values: number[]): void;
  clear(): void;
}

export interface ChartOptions {
  /** How many readings fit across; older ones fall off. */
  points?: number;
  /** One colour per series, in drawing order. */
  colors?: string[];
  /** Fixed top of the scale, or `auto` to grow to the tallest reading. */
  max?: number | 'auto';
  /** Fill under the first series. */
  fill?: boolean;
  /** Horizontal guides, as fractions of the height. */
  grid?: number[];
  height?: number;
  class?: string;
}

const NS = 'http://www.w3.org/2000/svg';

/**
 * A scrolling line graph.
 *
 * It is an SVG with a stretched view box and non-scaling strokes, so one
 * element is crisp at any width the layout gives it and at any display scale,
 * and a new reading costs one `d` attribute per series — no canvas, no
 * `devicePixelRatio` arithmetic, no redraw on resize.
 */
export function chart(options: ChartOptions = {}): Chart {
  const points = options.points ?? 60;
  const colors = options.colors ?? ['var(--accent)'];
  const grid = options.grid ?? [0.25, 0.5, 0.75];
  const svg = document.createElementNS(NS, 'svg');
  svg.setAttribute('class', `mon-chart${options.class ? ` ${options.class}` : ''}`);
  svg.setAttribute('viewBox', `0 0 ${points - 1} 100`);
  svg.setAttribute('preserveAspectRatio', 'none');
  svg.setAttribute('aria-hidden', 'true');
  if (options.height) svg.style.height = `${options.height}px`;
  for (const at of grid) {
    const line = document.createElementNS(NS, 'line');
    line.setAttribute('x1', '0');
    line.setAttribute('x2', String(points - 1));
    line.setAttribute('y1', String(at * 100));
    line.setAttribute('y2', String(at * 100));
    line.setAttribute('class', 'mon-grid');
    svg.appendChild(line);
  }
  const area = document.createElementNS(NS, 'path');
  if (options.fill !== false) {
    area.setAttribute('class', 'mon-area');
    area.setAttribute('fill', colors[0]);
    svg.appendChild(area);
  }
  const paths = colors.map((color) => {
    const path = document.createElementNS(NS, 'path');
    path.setAttribute('class', 'mon-line');
    path.setAttribute('stroke', color);
    path.setAttribute('fill', 'none');
    path.setAttribute('vector-effect', 'non-scaling-stroke');
    svg.appendChild(path);
    return path;
  });
  const series: number[][] = colors.map(() => []);
  const el = h('div', { class: 'mon-chart-box' });
  el.appendChild(svg);

  const draw = () => {
    const ceiling =
      options.max === 'auto' || options.max === undefined
        ? Math.max(1, ...series.flat().map((v) => (Number.isFinite(v) ? v : 0)))
        : options.max;
    const y = (v: number) => 100 - (Math.max(0, Math.min(ceiling, v)) / ceiling) * 100;
    // The newest reading sits at the right edge, so a graph that is not yet
    // full grows from the right rather than stretching to fit.
    const x = (i: number, n: number) => points - 1 - (n - 1 - i);
    series.forEach((values, s) => {
      if (values.length === 0) {
        paths[s].removeAttribute('d');
        return;
      }
      const d = values.map((v, i) => `${i === 0 ? 'M' : 'L'}${x(i, values.length).toFixed(2)} ${y(v).toFixed(2)}`).join(' ');
      paths[s].setAttribute('d', d);
      if (s === 0 && options.fill !== false) {
        area.setAttribute('d', `${d} L${(points - 1).toFixed(2)} 100 L${x(0, values.length).toFixed(2)} 100 Z`);
      }
    });
  };

  return {
    el,
    push(...values) {
      values.forEach((v, i) => {
        if (i >= series.length) return;
        series[i].push(Number.isFinite(v) ? v : 0);
        if (series[i].length > points) series[i].shift();
      });
      draw();
    },
    clear() {
      series.forEach((s) => (s.length = 0));
      draw();
    },
  };
}

/** A labelled bar: the shape every reading in the readout and the app wears. */
export function meter(label: string, options: { sub?: string; wide?: boolean } = {}): {
  el: HTMLElement;
  set(fraction: number, value: string, sub?: string): void;
} {
  const value = h('span', { class: 'mon-meter-value' }, '—');
  const sub = h('span', { class: 'mon-meter-sub' }, options.sub ?? '');
  const fill = h('i');
  const el = h(
    'div',
    { class: `mon-meter${options.wide ? ' wide' : ''}` },
    h('div', { class: 'mon-meter-head' }, h('span', { class: 'mon-meter-label' }, label), value),
    h('div', { class: 'mon-bar' }, fill),
    sub,
  );
  return {
    el,
    set(fraction, text, subText) {
      const pct = Math.max(0, Math.min(100, fraction * 100));
      fill.style.width = `${pct.toFixed(1)}%`;
      fill.style.background = heat(pct);
      value.textContent = text;
      if (subText !== undefined) {
        sub.textContent = subText;
        sub.hidden = !subText;
      }
    },
  };
}

/** The per-core grid: one column per hardware thread, tallest when busiest. */
export function coreGrid(): { el: HTMLElement; set(values: number[]): void } {
  const el = h('div', { class: 'mon-cores' });
  let bars: HTMLElement[] = [];
  return {
    el,
    set(values) {
      if (bars.length !== values.length) {
        bars = values.map((_, i) => h('div', { class: 'mon-core', title: `Thread ${i}` }, h('i')));
        el.replaceChildren(...bars);
        el.style.setProperty('--cores', String(values.length));
      }
      values.forEach((v, i) => {
        const fill = bars[i].firstElementChild as HTMLElement;
        fill.style.height = `${Math.max(2, Math.min(100, v))}%`;
        fill.style.background = heat(v);
        bars[i].title = `Thread ${i} — ${percent(v)}`;
      });
    },
  };
}

/** A number with its name under it, the way a dashboard states a fact. */
export function stat(label: string, initial = '—'): { el: HTMLElement; set(value: string, tone?: string): void } {
  const value = h('strong', { class: 'mon-stat-value' }, initial);
  const el = h('div', { class: 'mon-stat' }, value, h('span', { class: 'mon-stat-label' }, label));
  return {
    el,
    set(v, tone) {
      value.textContent = v;
      value.style.color = tone ?? '';
    },
  };
}
