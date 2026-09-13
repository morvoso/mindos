// The shapes the Task Manager is built from: the hero tile with its graph,
// the fact grid, the composition bar and the live table.
//
// They all update in place. A page that rebuilt its DOM every second would
// lose the text you were selecting, the row you had focused and the scroll
// position you had scrolled to — and would spend the whole second doing it.

import { h, reconcile } from '../dom';
import { icon } from '../icons';
import { chart, heat } from '../monitor';
import type { Chart, ChartOptions } from '../monitor';

/** A headline reading: a big number, a graph, and the facts behind it. */
export interface Hero {
  el: HTMLElement;
  chart: Chart;
  set(value: string, fraction?: number): void;
  setSub(text: string): void;
  setFacts(facts: [string, string][]): void;
}

export function hero(label: string, glyph: string, options: ChartOptions & { accent?: string } = {}): Hero {
  const accent = options.accent ?? 'var(--accent)';
  const value = h('strong', { class: 'tk-hero-value' }, '—');
  const sub = h('p', { class: 'tk-hero-sub' }, '');
  const facts = h('dl', { class: 'tk-facts' });
  const graph = chart({ points: 90, height: 84, colors: [accent], max: 100, ...options });
  const el = h(
    'section',
    { class: 'tk-hero' },
    h(
      'header',
      {},
      h('span', { class: 'tk-hero-mark', style: { color: accent } }, icon(glyph, 16)),
      h('h2', {}, label),
      value,
    ),
    sub,
    graph.el,
    facts,
  );
  el.style.setProperty('--tone', accent);
  return {
    el,
    chart: graph,
    set(text, fraction) {
      value.textContent = text;
      if (fraction !== undefined) value.style.color = heat(fraction * 100);
    },
    setSub(text) {
      sub.textContent = text;
      sub.hidden = !text;
    },
    setFacts(items) {
      setFacts(facts, items);
    },
  };
}

/** A label-and-value grid that keeps its rows between updates. */
export function factGrid(items: [string, string][] = [], cls = ''): { el: HTMLElement; set(items: [string, string][]): void } {
  const el = h('dl', { class: `tk-facts ${cls}`.trim() });
  setFacts(el, items);
  return { el, set: (next) => setFacts(el, next) };
}

function setFacts(el: HTMLElement, items: [string, string][]): void {
  reconcile(
    el,
    items,
    ([label]) => label,
    ([label]) => h('div', { class: 'tk-fact' }, h('dt', {}, label), h('dd', {}, '')),
    (node, [, value]) => {
      const dd = node.lastElementChild as HTMLElement;
      if (dd.textContent !== value) dd.textContent = value;
    },
  );
}

/**
 * One bar divided among named parts: how memory is actually spent, or how a
 * filesystem is. The parts keep their order and their colour, so the bar reads
 * the same way every time you look at it.
 */
export function composition(): { el: HTMLElement; set(parts: { label: string; value: number; color: string; hint?: string }[], total: number): void } {
  const bar = h('div', { class: 'tk-comp-bar' });
  const key = h('ul', { class: 'tk-comp-key' });
  const el = h('div', { class: 'tk-comp' }, bar, key);
  return {
    el,
    set(parts, total) {
      const scale = total > 0 ? 100 / total : 0;
      reconcile(
        bar,
        parts.filter((p) => p.value > 0),
        (p) => p.label,
        () => h('i', {}),
        (node, p) => {
          node.style.width = `${(p.value * scale).toFixed(2)}%`;
          node.style.background = p.color;
          node.title = `${p.label} — ${p.hint ?? ''}`;
        },
      );
      reconcile(
        key,
        parts,
        (p) => p.label,
        () => h('li', {}, h('i', {}), h('span', {}, ''), h('b', {}, '')),
        (node, p) => {
          (node.firstElementChild as HTMLElement).style.background = p.color;
          const [, name, value] = node.children as unknown as HTMLElement[];
          name.textContent = p.label;
          value.textContent = p.hint ?? '';
        },
      );
    },
  };
}

/** A single labelled bar with a value on the right — a filesystem, a card's VRAM. */
export function barRow(): { el: HTMLElement; set(label: string, sub: string, fraction: number, value: string): void } {
  const name = h('span', { class: 'tk-barrow-name' }, '');
  const detail = h('span', { class: 'tk-barrow-sub' }, '');
  const value = h('span', { class: 'tk-barrow-value' }, '');
  const fill = h('i');
  const el = h(
    'div',
    { class: 'tk-barrow' },
    h('div', { class: 'tk-barrow-head' }, name, detail, value),
    h('div', { class: 'tk-bar' }, fill),
  );
  return {
    el,
    set(label, sub, fraction, text) {
      name.textContent = label;
      detail.textContent = sub;
      value.textContent = text;
      const pct = Math.max(0, Math.min(100, fraction * 100));
      fill.style.width = `${pct.toFixed(1)}%`;
      fill.style.background = heat(pct);
    },
  };
}

export interface Column<T> {
  key: string;
  label: string;
  /** A grid track: `1fr`, `90px`, `minmax(0,2fr)`. */
  width: string;
  align?: 'start' | 'end';
  /** What the cell shows. Text is set directly; an element replaces the cell. */
  cell(row: T): string | HTMLElement;
  /**
   * Paint the cell in place instead, for a cell with structure in it. Rebuilding
   * an element per row per tick is what `cell` returning an element costs; a
   * column that does it hundreds of times a second should write into the cell
   * it already has.
   */
  render?(cell: HTMLElement, row: T): void;
  /** A tone for the cell's text, when the number should carry one. */
  tone?(row: T): string | undefined;
  sortable?: boolean;
  title?: string;
}

export interface TableOptions<T> {
  /** The column currently sorted by, and which way. */
  sort?: { key: string; ascending: boolean };
  onSort?(key: string): void;
  onActivate?(row: T): void;
  onSelect?(row: T | undefined): void;
  onContext?(row: T, at: { x: number; y: number }): void;
  empty?: string;
  class?: string;
}

export interface DataTable<T> {
  el: HTMLElement;
  set(rows: T[]): void;
  setSort(sort: { key: string; ascending: boolean }): void;
  selected(): T | undefined;
  select(key: string | undefined): void;
}

/**
 * The live table. Rows are matched by key and updated in place, so a table
 * refreshing once a second neither flickers nor loses the row under the
 * pointer; only the cells whose text actually changed are written.
 */
export function dataTable<T>(columns: Column<T>[], keyOf: (row: T) => string, options: TableOptions<T> = {}): DataTable<T> {
  const track = columns.map((c) => c.width).join(' ');
  const head = h('div', { class: 'tk-thead', role: 'row' });
  const body = h('div', { class: 'tk-tbody', role: 'rowgroup' });
  const empty = h('p', { class: 'tk-empty', hidden: true }, options.empty ?? 'Nothing to show.');
  const el = h('div', { class: `tk-table ${options.class ?? ''}`.trim(), role: 'table' }, head, body, empty);
  head.style.gridTemplateColumns = track;
  const heads = new Map<string, HTMLElement>();
  for (const c of columns) {
    const arrow = h('i', { class: 'tk-sort' });
    const cell = c.sortable
      ? h('button', { class: 'tk-th', type: 'button', title: c.title ?? `Sort by ${c.label.toLowerCase()}`, onclick: () => options.onSort?.(c.key) }, h('span', {}, c.label), arrow)
      : h('span', { class: 'tk-th' }, c.label);
    if (c.align === 'end') cell.classList.add('end');
    heads.set(c.key, cell);
    head.appendChild(cell);
  }
  let rows: T[] = [];
  let current: string | undefined;

  const applySelection = () => {
    for (const node of body.children) {
      (node as HTMLElement).classList.toggle('on', (node as HTMLElement).dataset.key === current);
    }
  };

  const build = (row: T) => {
    const node = h('div', { class: 'tk-tr', role: 'row', tabindex: -1 });
    node.style.gridTemplateColumns = track;
    for (const c of columns) {
      const cell = h('span', { class: `tk-td${c.align === 'end' ? ' end' : ''}`, role: 'cell' });
      node.appendChild(cell);
    }
    node.addEventListener('click', () => {
      current = keyOf(row);
      applySelection();
      options.onSelect?.(rows.find((r) => keyOf(r) === current));
    });
    node.addEventListener('dblclick', () => options.onActivate?.(row));
    node.addEventListener('contextmenu', (e) => {
      if (!options.onContext) return;
      e.preventDefault();
      current = keyOf(row);
      applySelection();
      const found = rows.find((r) => keyOf(r) === current);
      if (found) options.onContext(found, { x: e.clientX, y: e.clientY });
    });
    return node;
  };

  const paint = (node: HTMLElement, row: T) => {
    columns.forEach((c, i) => {
      const cell = node.children[i] as HTMLElement;
      if (c.render) {
        c.render(cell, row);
        return;
      }
      const value = c.cell(row);
      if (typeof value === 'string') {
        if (cell.textContent !== value) cell.textContent = value;
      } else if (cell.firstElementChild !== value) {
        cell.replaceChildren(value);
      }
      const tone = c.tone?.(row);
      if (cell.style.color !== (tone ?? '')) cell.style.color = tone ?? '';
    });
  };

  const setSort = (sort: { key: string; ascending: boolean }) => {
    for (const [key, cell] of heads) {
      const on = key === sort.key;
      cell.classList.toggle('sorted', on);
      cell.setAttribute('aria-sort', on ? (sort.ascending ? 'ascending' : 'descending') : 'none');
      const arrow = cell.querySelector<HTMLElement>('.tk-sort');
      if (arrow) arrow.textContent = on ? (sort.ascending ? '▲' : '▼') : '';
    }
  };
  if (options.sort) setSort(options.sort);

  return {
    el,
    set(next) {
      rows = next;
      reconcile(body, next, keyOf, build, paint);
      empty.hidden = next.length > 0;
      applySelection();
    },
    setSort,
    selected: () => rows.find((r) => keyOf(r) === current),
    select(key) {
      current = key;
      applySelection();
    },
  };
}

/** A card with a title, for a page that is a stack of them. */
export function panel(title: string, sub?: string, ...body: (HTMLElement | null)[]): HTMLElement {
  return h(
    'section',
    { class: 'tk-panel' },
    h('header', { class: 'tk-panel-head' }, h('h2', {}, title), sub ? h('span', { class: 'tk-panel-sub' }, sub) : null),
    ...body,
  );
}

/** A short status word with the colour that matches what it says. */
export function state(text: string, tone: 'ok' | 'warn' | 'danger' | 'idle'): HTMLElement {
  return h('span', { class: `tk-state ${tone}` }, text);
}
