// Bits shared by the Settings app (and once the Files app): the window frame, form rows,
// dialogs and formatting helpers.

import * as bridge from '../bridge';
import { h } from '../dom';
import { glassLayer } from '../glass';
import { icon } from '../icons';
import { store } from '../state';

export interface NavItem {
  id: string;
  label: string;
  icon: string;
}

export interface AppFrame {
  side: HTMLElement;
  content: HTMLElement;
  setActive(id: string): void;
}

/** The standard app window: a sidebar with the app's name and pages, and a content column. */
export function appFrame(root: HTMLElement, title: string, items: NavItem[], onNav: (id: string) => void): AppFrame {
  const nav = h('nav', { class: 'app-nav' });
  const buttons = new Map<string, HTMLElement>();
  for (const it of items) {
    const b = h('button', { class: 'nav-item', onclick: () => onNav(it.id) }, h('span', { class: 'nav-ic' }, icon(it.icon, 17)), h('span', {}, it.label));
    buttons.set(it.id, b);
    nav.appendChild(b);
  }
  const side = h('aside', { class: 'app-side' }, h('div', { class: 'app-brand' }, h('span', { class: 'app-brand-mark' }, icon('mind', 18)), h('span', { class: 'app-brand-text' }, title)), nav);
  const content = h('main', { class: 'app-content' });
  root.append(side, content);
  frostSidebar(side);
  return {
    side,
    content,
    setActive(id) {
      for (const [k, b] of buttons) b.classList.toggle('on', k === id);
    },
  };
}

/**
 * Frost an app sidebar with the wallpaper. An app window does not know where
 * it is on the screen, so the crop is the wallpaper's left edge; it still
 * ties the window to the desktop behind it.
 */
export function frostSidebar(side: HTMLElement): void {
  const out = store.state.outputs[0];
  if (!out) return;
  glassLayer(side, { output: out.name, origin: () => ({ x: 0, y: Math.round(out.height * 0.25) }) });
}

export function pageHeader(title: string, sub?: string, ...extra: (HTMLElement | null)[]): HTMLElement {
  return h('header', { class: 'page-head' }, h('div', { class: 'page-titles' }, h('h1', { class: 'page-title' }, title), sub ? h('p', { class: 'page-sub' }, sub) : null), ...extra);
}

export function card(title: string | null, ...children: (HTMLElement | string | null | false)[]): HTMLElement {
  return h('section', { class: 'card' }, title ? h('h2', { class: 'card-title' }, title) : null, ...children);
}

export function row(label: string, help: string | null, ...controls: (HTMLElement | null)[]): HTMLElement {
  return h('div', { class: 'row' }, h('div', { class: 'row-text' }, h('div', { class: 'row-label' }, label), help ? h('div', { class: 'row-help' }, help) : null), h('div', { class: 'row-ctl' }, ...controls));
}

export function toggle(checked: boolean, onChange: (v: boolean) => void, disabled = false): HTMLElement {
  const input = h('input', { type: 'checkbox', checked, disabled }) as HTMLInputElement;
  input.addEventListener('change', () => onChange(input.checked));
  return h('label', { class: 'switch' }, input, h('i'));
}

export function selectBox<T extends string | number>(options: { value: T; label: string }[], value: T | undefined, onChange: (v: T) => void): HTMLSelectElement {
  const sel = h('select', { class: 'select' }) as HTMLSelectElement;
  for (const o of options) sel.appendChild(h('option', { value: String(o.value), selected: o.value === value }, o.label));
  sel.addEventListener('change', () => {
    const raw = sel.value;
    const opt = options.find((o) => String(o.value) === raw);
    if (opt) onChange(opt.value);
  });
  return sel;
}

export function pill(text: string, cls = ''): HTMLElement {
  return h('span', { class: `pill ${cls}`.trim() }, text);
}

export function progress(fraction: number, cls = ''): HTMLElement {
  const bar = h('div', { class: `progress ${cls}`.trim() }, h('i', { style: { width: `${Math.round(Math.max(0, Math.min(1, fraction)) * 100)}%` } }));
  return bar;
}

/** A status line that shows a message for a while. */
export function notice(): { el: HTMLElement; show(text: string, kind?: 'ok' | 'error' | 'info'): void; clear(): void } {
  const el = h('div', { class: 'notice', hidden: true });
  let timer: ReturnType<typeof setTimeout> | undefined;
  return {
    el,
    show(text, kind = 'info') {
      el.textContent = text;
      el.dataset.kind = kind;
      el.hidden = false;
      if (timer) clearTimeout(timer);
      if (kind !== 'error') timer = setTimeout(() => (el.hidden = true), 6000);
    },
    clear() {
      el.hidden = true;
    },
  };
}

/** A modal sheet inside the app window (there are no popups in app windows). */
export function dialog(root: HTMLElement, title: string, body: HTMLElement, actions: HTMLElement[]): () => void {
  const sheet = h('div', { class: 'sheet', role: 'dialog' }, h('div', { class: 'sheet-title' }, title), body, h('div', { class: 'sheet-actions' }, ...actions));
  const backdrop = h('div', { class: 'sheet-backdrop' }, sheet);
  const close = () => {
    backdrop.remove();
    window.removeEventListener('keydown', onKey, true);
  };
  const onKey = (e: KeyboardEvent) => {
    if (e.key === 'Escape') {
      e.preventDefault();
      e.stopPropagation();
      close();
    }
  };
  backdrop.addEventListener('pointerdown', (e) => {
    if (e.target === backdrop) close();
  });
  window.addEventListener('keydown', onKey, true);
  root.appendChild(backdrop);
  requestAnimationFrame(() => backdrop.classList.add('in'));
  return close;
}

export function fmtBytes(n: number): string {
  if (!Number.isFinite(n) || n < 0) return '';
  if (n < 1024) return `${n} B`;
  const units = ['KB', 'MB', 'GB', 'TB'];
  let v = n / 1024;
  let i = 0;
  while (v >= 1024 && i < units.length - 1) {
    v /= 1024;
    i++;
  }
  return `${v < 10 ? v.toFixed(1) : Math.round(v)} ${units[i]}`;
}

export function fmtDate(secs: number): string {
  if (!secs) return '';
  const d = new Date(secs * 1000);
  const now = new Date();
  const sameDay = d.toDateString() === now.toDateString();
  const time = `${String(d.getHours()).padStart(2, '0')}:${String(d.getMinutes()).padStart(2, '0')}`;
  if (sameDay) return `Today ${time}`;
  const y = d.getFullYear() === now.getFullYear() ? '' : ` ${d.getFullYear()}`;
  return `${d.getDate()} ${['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec'][d.getMonth()]}${y} ${time}`;
}

/** URL of a host-generated thumbnail for an image on disk. */
export function thumbUrl(path: string, width: number): string {
  return `mindos://shell/thumb/${encodeURIComponent(path).replace(/%2F/g, '/')}?w=${width}`;
}

export function fileUrl(path: string): string {
  return `mindos://shell/file/${encodeURIComponent(path).replace(/%2F/g, '/')}`;
}

export function openApp(name: string, page?: string, arg?: string): void {
  bridge.send('shell.openApp', { name, page, arg });
}

export function setTitle(title: string): void {
  document.title = title;
  bridge.send('app.setTitle', { title });
}
