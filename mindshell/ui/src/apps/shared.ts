// Bits shared by the Settings app (and once the Files app): the window frame, form rows,
// dialogs and formatting helpers.

import * as bridge from '../bridge';
import { h, newId } from '../dom';
import { glassLayer } from '../glass';
import { icon } from '../icons';
import { store } from '../state';

export interface NavItem {
  id: string;
  label: string;
  icon: string;
  group?: string;
  keywords?: string;
}

export interface AppFrame {
  side: HTMLElement;
  content: HTMLElement;
  dispose(): void;
  setActive(id: string): void;
}

/** The standard app window: a sidebar with the app's name and pages, and a content column. */
export function appFrame(root: HTMLElement, title: string, items: NavItem[], onNav: (id: string) => void): AppFrame {
  const nav = h('nav', { class: 'app-nav', 'aria-label': 'Settings pages' });
  const buttons = new Map<string, HTMLButtonElement>();
  const groups = new Map<string, HTMLElement>();
  for (const it of items) {
    const group = it.group ?? '';
    if (!groups.has(group)) {
      const section = h('div', { class: 'nav-group' }, group ? h('div', { class: 'nav-group-label' }, group) : null);
      groups.set(group, section);
      nav.append(section);
    }
    const b = h('button', { class: 'nav-item', type: 'button', onclick: () => onNav(it.id) }, h('span', { class: 'nav-ic', 'aria-hidden': 'true' }, icon(it.icon, 17)), h('span', {}, it.label));
    buttons.set(it.id, b);
    groups.get(group)!.appendChild(b);
  }
  const search = h('input', { type: 'search', class: 'nav-search-input', placeholder: 'Find a setting', 'aria-label': 'Find a setting', autocomplete: 'off', spellcheck: false });
  const empty = h('p', { class: 'nav-empty', hidden: true, role: 'status' }, 'No matching settings. Try “GPU”, “Wi-Fi” or “display”.');
  const filter = () => {
    const words = search.value.trim().toLowerCase().split(/\s+/);
    for (const item of items) {
      const text = `${item.label} ${item.group ?? ''} ${item.keywords ?? ''}`.toLowerCase();
      buttons.get(item.id)!.hidden = !words.every((word) => text.includes(word));
    }
    for (const section of groups.values()) section.hidden = ![...section.querySelectorAll<HTMLButtonElement>('button')].some((b) => !b.hidden);
    empty.hidden = [...buttons.values()].some((b) => !b.hidden);
  };
  search.addEventListener('input', filter);
  search.addEventListener('keydown', (e) => {
    if (e.key === 'Escape') { search.value = ''; filter(); e.stopPropagation(); }
    if (e.key === 'Enter' || e.key === 'ArrowDown') {
      const first = [...buttons.values()].find((b) => !b.hidden);
      if (first) { e.preventDefault(); first.focus(); if (e.key === 'Enter') first.click(); }
    }
  });
  nav.addEventListener('keydown', (e) => {
    if (!['ArrowUp', 'ArrowDown', 'Home', 'End'].includes(e.key)) return;
    const visible = [...buttons.values()].filter((b) => !b.hidden);
    const i = visible.indexOf(document.activeElement as HTMLButtonElement);
    if (i < 0) return;
    e.preventDefault();
    const next = e.key === 'Home' ? 0 : e.key === 'End' ? visible.length - 1 : (i + (e.key === 'ArrowDown' ? 1 : -1) + visible.length) % visible.length;
    visible[next]?.focus();
  });
  root.addEventListener('keydown', (e) => {
    if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 'k') {
      e.preventDefault(); search.focus(); search.select();
    }
  });
  const side = h('aside', { class: 'app-side' }, h('div', { class: 'app-brand' }, h('span', { class: 'app-brand-mark' }, icon('mind', 20)), h('span', { class: 'app-brand-copy' }, h('span', { class: 'app-brand-text' }, 'MindOS'), h('span', { class: 'app-brand-caption' }, title))),
    h('div', { class: 'nav-search' }, icon('search', 15), search), nav, empty,
    h('div', { class: 'nav-foot' }, h('kbd', {}, 'Ctrl K'), ' Find settings'));
  const content = h('main', { class: 'app-content', tabindex: -1 });
  root.append(side, content);
  const disposeGlass = frostSidebar(side);
  return {
    side,
    content,
    dispose: disposeGlass,
    setActive(id) {
      for (const [k, b] of buttons) {
        b.classList.toggle('on', k === id);
        if (k === id) b.setAttribute('aria-current', 'page');
        else b.removeAttribute('aria-current');
      }
    },
  };
}

/**
 * Frost an app sidebar with the wallpaper. An app window does not know where
 * it is on the screen, so the crop is the wallpaper's left edge; it still
 * ties the window to the desktop behind it.
 */
export function frostSidebar(side: HTMLElement): () => void {
  const out = store.state.outputs[0];
  if (!out) return () => {};
  return glassLayer(side, { output: out.name, origin: () => ({ x: 0, y: Math.round(out.height * 0.25) }) }).dispose;
}

export function pageHeader(title: string, sub?: string, ...extra: (HTMLElement | null)[]): HTMLElement {
  return h('header', { class: 'page-head' }, h('div', { class: 'page-titles' }, h('h1', { class: 'page-title' }, title), sub ? h('p', { class: 'page-sub' }, sub) : null), ...extra);
}

export function card(title: string | null, ...children: (HTMLElement | string | null | false)[]): HTMLElement {
  return h('section', { class: 'card' }, title ? h('h2', { class: 'card-title' }, title) : null, ...children);
}

export function row(label: string, help: string | null, ...controls: (HTMLElement | null)[]): HTMLElement {
  const labelId = newId('label');
  const helpId = newId('help');
  for (const control of controls) {
    if (!control) continue;
    const inputs = control.matches('input, select, textarea') ? [control] : [...control.querySelectorAll('input, select, textarea')];
    for (const input of inputs) {
      if (!input.hasAttribute('aria-label') && !input.hasAttribute('aria-labelledby')) input.setAttribute('aria-labelledby', labelId);
      if (help && !input.hasAttribute('aria-describedby')) input.setAttribute('aria-describedby', helpId);
    }
  }
  return h('div', { class: 'row' }, h('div', { class: 'row-text' }, h('div', { class: 'row-label', id: labelId }, label), help ? h('div', { class: 'row-help', id: helpId }, help) : null), h('div', { class: 'row-ctl' }, ...controls));
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
export function notice(): { el: HTMLElement; show(text: string, kind?: 'ok' | 'error' | 'info', durationMs?: number): void; clear(): void } {
  const el = h('div', { class: 'notice', hidden: true, role: 'status', 'aria-live': 'polite', 'aria-atomic': 'true' });
  let timer: ReturnType<typeof setTimeout> | undefined;
  return {
    el,
    show(text, kind = 'info', durationMs = 6000) {
      el.textContent = text;
      el.dataset.kind = kind;
      el.hidden = false;
      if (timer) clearTimeout(timer);
      if (kind !== 'error' && durationMs > 0) timer = setTimeout(() => (el.hidden = true), durationMs);
    },
    clear() {
      if (timer) clearTimeout(timer);
      el.hidden = true;
    },
  };
}

/** A modal sheet inside the app window (there are no popups in app windows). */
export function dialog(root: HTMLElement, title: string, body: HTMLElement, actions: HTMLElement[]): () => void {
  const titleId = newId('dialog-title');
  const previousFocus = document.activeElement as HTMLElement | null;
  const sheet = h('div', { class: 'sheet', role: 'dialog', 'aria-modal': 'true', 'aria-labelledby': titleId, tabindex: -1 }, h('div', { class: 'sheet-title', id: titleId }, title), body, h('div', { class: 'sheet-actions' }, ...actions));
  const backdrop = h('div', { class: 'sheet-backdrop' }, sheet);
  const behind = [...root.children].filter((node): node is HTMLElement => node instanceof HTMLElement);
  const wasInert = behind.map((node) => node.inert);
  behind.forEach((node) => { node.inert = true; });
  let closed = false;
  const close = () => {
    if (closed) return;
    closed = true;
    backdrop.remove();
    behind.forEach((node, i) => { node.inert = wasInert[i]; });
    window.removeEventListener('keydown', onKey, true);
    if (previousFocus?.isConnected) previousFocus.focus();
  };
  const onKey = (e: KeyboardEvent) => {
    if (e.key === 'Escape') {
      e.preventDefault();
      e.stopPropagation();
      close();
    }
    if (e.key === 'Tab') {
      const focusable = [...sheet.querySelectorAll<HTMLElement>('button:not(:disabled), input:not(:disabled), select:not(:disabled), textarea:not(:disabled), a[href], [tabindex="0"]')].filter((node) => node.getClientRects().length > 0);
      const first = focusable[0], last = focusable.at(-1);
      if (!first) { e.preventDefault(); sheet.focus(); }
      else if (e.shiftKey && (document.activeElement === first || document.activeElement === sheet)) { e.preventDefault(); last!.focus(); }
      else if (!e.shiftKey && (document.activeElement === last || document.activeElement === sheet)) { e.preventDefault(); first.focus(); }
    }
  };
  backdrop.addEventListener('pointerdown', (e) => {
    if (e.target === backdrop) close();
  });
  window.addEventListener('keydown', onKey, true);
  root.appendChild(backdrop);
  requestAnimationFrame(() => {
    if (closed) return;
    backdrop.classList.add('in');
    (sheet.querySelector<HTMLElement>('input:not(:disabled), select:not(:disabled), button:not(:disabled)') ?? sheet).focus();
  });
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
