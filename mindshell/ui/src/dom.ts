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

/** A repeating timer that stops when the element leaves the document. It ticks
 *  slower (or not at all) while a game runs — see quiet.ts. */
export function every(el: Element, ms: number, fn: () => void): () => void {
  let id: ReturnType<typeof setInterval> | undefined;
  const tick = () => {
    if (!el.isConnected) return stop();
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
    if (!isQuiet()) fn();
  });
  const stop = () => {
    if (id !== undefined) clearInterval(id);
    id = undefined;
    offQuiet();
  };
  fn();
  arm();
  return stop;
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
