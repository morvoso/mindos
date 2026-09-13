// Tiny DOM helpers: h() builds elements, plus a few utilities used everywhere.

import { isQuiet, onQuiet, quietInterval } from './quiet';

type Child = Node | string | number | null | undefined | false;
type Attrs = Record<string, unknown> | null | undefined;

export function h<K extends keyof HTMLElementTagNameMap>(tag: K, attrs?: Attrs, ...children: Child[]): HTMLElementTagNameMap[K] {
  const el = document.createElement(tag);
  if (attrs) {
    for (const [key, value] of Object.entries(attrs)) {
      if (value === null || value === undefined || value === false) continue;
      if (key === 'class') el.className = String(value);
      else if (key === 'style') {
        if (typeof value === 'object') Object.assign(el.style, value as Record<string, string>);
        else el.setAttribute('style', String(value));
      }
      else if (key === 'dataset') Object.assign(el.dataset, value as Record<string, string>);
      else if (key.startsWith('on') && typeof value === 'function') el.addEventListener(key.slice(2).toLowerCase(), value as EventListener);
      else if (key in el && typeof value !== 'string') (el as unknown as Record<string, unknown>)[key] = value;
      else el.setAttribute(key, value === true ? '' : String(value));
    }
  }
  append(el, children);
  return el;
}

export function append(el: Node, children: Child[]): void {
  for (const c of children) {
    if (c === null || c === undefined || c === false) continue;
    el.appendChild(typeof c === 'object' ? c : document.createTextNode(String(c)));
  }
}

export function clear(el: Node): void {
  while (el.firstChild) el.removeChild(el.firstChild);
}

export function clamp(v: number, lo: number, hi: number): number {
  return Math.max(lo, Math.min(hi, v));
}

export function pad2(n: number): string {
  return n < 10 ? '0' + n : String(n);
}

export function debounce<A extends unknown[]>(fn: (...a: A) => void, ms: number): (...a: A) => void {
  let t: ReturnType<typeof setTimeout> | undefined;
  return (...a: A) => {
    if (t) clearTimeout(t);
    t = setTimeout(() => fn(...a), ms);
  };
}

/** Whether `el` is actually drawn. `display:none` anywhere above it — a panel
 *  that is put away, a home screen the windows have covered — means nothing it
 *  samples can be seen; `visibility:hidden` and `position:fixed` still count as
 *  drawn, which is why the first test is not enough on its own. */
function drawn(el: Element): boolean {
  const box = el as HTMLElement;
  return box.offsetParent !== null || box.offsetWidth > 0 || box.offsetHeight > 0 || box.getClientRects().length > 0;
}

/** Something that was off screen is back: any sampler that skipped a tick for
 *  that reason should take one now rather than wait out its interval. */
export const RESUME_EVENT = 'shell.resume';

/** A repeating timer that stops when the element leaves the document. It ticks
 *  slower (or not at all) while a game runs — see quiet.ts — and not at all
 *  while the page is hidden or the element is not being drawn: a tick missed
 *  that way runs as soon as it can be seen again. The first call is immediate. */
export function every(el: Element, ms: number, fn: () => void): () => void {
  let id: ReturnType<typeof setInterval> | undefined;
  let missed = false;
  const tick = () => {
    if (!el.isConnected) return stop();
    if (document.hidden || !drawn(el)) {
      missed = true;
      return;
    }
    missed = false;
    fn();
  };
  const arm = () => {
    if (id !== undefined) clearInterval(id);
    id = undefined;
    const wait = quietInterval(ms);
    if (wait > 0) id = setInterval(tick, wait);
  };
  const offQuiet = onQuiet(() => {
    if (!el.isConnected) return stop();
    arm();
    if (!isQuiet()) tick();
  });
  const shown = () => {
    if (!el.isConnected) return stop();
    if (!document.hidden && missed) tick();
  };
  document.addEventListener('visibilitychange', shown);
  window.addEventListener(RESUME_EVENT, shown);
  const stop = () => {
    if (id !== undefined) clearInterval(id);
    id = undefined;
    offQuiet();
    document.removeEventListener('visibilitychange', shown);
    window.removeEventListener(RESUME_EVENT, shown);
  };
  fn();
  arm();
  return stop;
}

/** Mark `el` with `offscreen` while it is scrolled out of view (or the page is
 *  hidden), so an endless animation on it can hold still — see app.css. */
let onScreen: IntersectionObserver | undefined;
export function watchOnScreen(el: Element): void {
  if (typeof IntersectionObserver !== 'function') return;
  onScreen ??= new IntersectionObserver((entries) => {
    for (const e of entries) e.target.classList.toggle('offscreen', !e.isIntersecting);
  });
  onScreen.observe(el);
}

let uid = 0;
export function newId(prefix: string): string {
  uid += 1;
  return `${prefix}-${Date.now().toString(36)}${uid.toString(36)}`;
}

export function deepClone<T>(v: T): T {
  return JSON.parse(JSON.stringify(v)) as T;
}

/** Format bytes as GiB with one decimal. */
export function gib(bytes: number): string {
  return (bytes / 1073741824).toFixed(1);
}

export function formatUptime(seconds: number): string {
  const d = Math.floor(seconds / 86400);
  const hh = Math.floor((seconds % 86400) / 3600);
  const mm = Math.floor((seconds % 3600) / 60);
  return d > 0 ? `${d}d ${hh}h` : hh > 0 ? `${hh}h ${pad2(mm)}m` : `${mm}m`;
}

/** Keep `container`'s children in sync with `items` by key, creating, moving and removing as needed. */
export function reconcile<T>(
  container: HTMLElement,
  items: T[],
  key: (item: T) => string,
  create: (item: T) => HTMLElement,
  update?: (el: HTMLElement, item: T) => void,
  remove?: (el: HTMLElement) => void,
): void {
  const existing = new Map<string, HTMLElement>();
  for (const child of Array.from(container.children)) {
    const k = (child as HTMLElement).dataset.key;
    if (k !== undefined) existing.set(k, child as HTMLElement);
  }
  const keep = new Set<string>();
  let cursor: Element | null = container.firstElementChild;
  for (const item of items) {
    const k = key(item);
    keep.add(k);
    let el = existing.get(k);
    if (!el) {
      el = create(item);
      el.dataset.key = k;
    }
    update?.(el, item);
    if (el === cursor) {
      cursor = cursor.nextElementSibling;
    } else {
      container.insertBefore(el, cursor);
    }
  }
  for (const [k, el] of existing) {
    if (!keep.has(k)) {
      remove?.(el);
      el.remove();
    }
  }
}

/**
 * Hold the reader's place across an update.
 *
 * WebKit implements no scroll anchoring. When a live page grows or shrinks
 * above the viewport — a process appears, a warning opens, a number gets wide
 * enough to wrap — everything below it slides out from under the eye, and a
 * page that shrank for a single frame comes back scrolled somewhere else
 * entirely, because the browser clamps the offset on the way down and never
 * gives it back. This notes what was under the top edge, runs the update, and
 * puts that back where it was.
 *
 * Two things it is careful not to do. It measures with `offsetTop`, which is
 * where layout put the element and not where the eye finds it, so a graph
 * whose line moved and a bar that grew are not mistaken for a shift. And it
 * will not follow a row that moved because the list re-sorted: a table sorted
 * by processor share reorders under you constantly, and chasing one row
 * through it would drag the whole page along. When the deepest anchor cannot
 * be trusted the next one out is used, and the offset is held instead.
 */
export function anchored<T>(inside: Element, update: () => T): T {
  const scroller = scrollParent(inside);
  if (!scroller || scroller.scrollTop <= 0) return update();
  const was = scroller.scrollTop;
  const path = anchorPath(scroller);
  const result = update();
  for (let i = path.length - 1; i >= 0; i -= 1) {
    const level = path[i];
    const parent = level.el.parentElement;
    if (!parent || !level.el.isConnected || !level.el.offsetParent) continue;
    const moved = Math.abs(indexIn(parent, level.el) - level.index);
    const churn = Math.abs(parent.childElementCount - level.count);
    // Further than the arrivals and departures explain: the list reordered,
    // and the layout under it did not move at all.
    if (moved > churn) continue;
    scroller.scrollTop = was + (level.el.offsetTop - level.top);
    return result;
  }
  // Nothing left to measure against; at least undo a momentary shrink's clamp.
  scroller.scrollTop = was;
  return result;
}

/** One step of the walk down to what the reader is looking at. */
interface Anchor {
  el: HTMLElement;
  /** Where layout had it, and where it sat among its siblings. */
  top: number;
  index: number;
  count: number;
}

/** The nearest ancestor that actually scrolls, if there is one. */
function scrollParent(el: Element): HTMLElement | undefined {
  let node: Element | null = el;
  while (node instanceof HTMLElement) {
    if (node.scrollHeight > node.clientHeight && /auto|scroll|overlay/.test(getComputedStyle(node).overflowY)) return node;
    node = node.parentElement;
  }
  return undefined;
}

/**
 * The elements under the top edge, outermost first. The walk stops at a keyed
 * node — a reconciled row outlives an update by definition, its contents may
 * not — and never steps into something with nothing inside it, because a leaf
 * is where the drawing lives: an SVG path, the fill of a bar. Those move with
 * the reading, not with the layout.
 */
function anchorPath(scroller: HTMLElement): Anchor[] {
  const y = scroller.getBoundingClientRect().top + 1;
  const path: Anchor[] = [];
  let node: HTMLElement = scroller;
  for (let depth = 0; depth < 8; depth += 1) {
    const next = firstPast(node, y);
    if (!next || next.childElementCount === 0) break;
    path.push({ el: next, top: next.offsetTop, index: indexIn(node, next), count: node.childElementCount });
    node = next;
    if (next.dataset.key !== undefined) break;
  }
  return path;
}

function indexIn(parent: Element, child: Element): number {
  const kids = parent.children;
  for (let i = 0; i < kids.length; i += 1) if (kids[i] === child) return i;
  return -1;
}

/** The first child whose bottom edge is past `y`, without measuring a long list. */
function firstPast(el: HTMLElement, y: number): HTMLElement | undefined {
  const kids = el.children;
  const n = kids.length;
  if (n > 24 && kids[0] instanceof HTMLElement) {
    // A long list is stacked in order, so the crossing can be searched for.
    let lo = 0;
    let hi = n - 1;
    while (lo < hi) {
      const mid = (lo + hi) >> 1;
      if (kids[mid].getBoundingClientRect().bottom > y) hi = mid;
      else lo = mid + 1;
    }
    const found = kids[lo];
    return found instanceof HTMLElement && found.getBoundingClientRect().bottom > y ? found : undefined;
  }
  for (const kid of kids) {
    if (!(kid instanceof HTMLElement)) continue;
    const box = kid.getBoundingClientRect();
    if (box.width === 0 && box.height === 0) continue;
    if (box.bottom > y) return kid;
  }
  return undefined;
}
