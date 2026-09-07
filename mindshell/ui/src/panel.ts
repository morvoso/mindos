// A panel window: the bar with its widgets plus, in edit mode, the settings strip.

import * as actions from './actions';
import * as bridge from './bridge';
import { clamp, debounce, h, reconcile } from './dom';
import { EDIT_EXTRA, isVertical, panelWindowRect, rectIn } from './geometry';
import { glassLayer } from './glass';
import { icon } from './icons';
import { isFitPanel, panelById } from './layout';
import { store } from './state';
import { allWidgets, getWidget, mergedConfig, type WidgetCtx, type WidgetInstance } from './widgets/registry';
import type { Align, Anchor, Edge, MenuAction, PanelDef, PanelLayer, WidgetEntry } from './types';

interface Mounted {
  ctx: WidgetCtx;
  inst: WidgetInstance;
  slot: HTMLElement;
}

/** Island or flush: the layout's say, else thick panels float (app.css inset rule). */
function panelFloats(p: PanelDef): boolean {
  if (typeof p.float === 'boolean') return p.float;
  return p.size > 30 || isFitPanel(p);
}

const EDGES: { edge: Edge; icon: string; label: string }[] = [
  { edge: 'top', icon: 'chevron-up', label: 'Top' },
  { edge: 'bottom', icon: 'chevron-down', label: 'Bottom' },
  { edge: 'left', icon: 'chevron-left', label: 'Left' },
  { edge: 'right', icon: 'chevron-right', label: 'Right' },
];

export function renderPanel(root: HTMLElement, panelId: string, output: string): () => void {
  root.classList.add('panel-window');
  const glow = h('div', { class: 'panel-glow' });
  const widgetsEl = h('div', { class: 'panel-widgets' });
  const dropInd = h('div', { class: 'drop-ind', hidden: true });
  // The island is the visible bar, inset inside the window (app.css); the
  // glass under it shows the wallpaper at the island's screen position.
  const island = h('div', { class: 'panel-island' }, glow, widgetsEl, dropInd);
  const bar = h('div', { class: 'panel-bar' }, island);
  const strip = h('div', { class: 'panel-strip', hidden: true });
  root.append(strip, bar);

  const mounted = new Map<string, Mounted>();
  let current: PanelDef | undefined;

  // At rest the bar steps back (app.css `.rest`): a few seconds after the
  // pointer leaves, unless a popup opened from it is still up.
  let restTimer: ReturnType<typeof setTimeout> | undefined;
  const wake = () => {
    if (restTimer) clearTimeout(restTimer);
    restTimer = undefined;
    root.classList.remove('rest');
  };
  const settle = () => {
    if (restTimer) clearTimeout(restTimer);
    restTimer = setTimeout(() => {
      restTimer = undefined;
      if (!root.matches(':hover') && !root.querySelector('.w.open, .task.open')) root.classList.add('rest');
    }, 2500);
  };
  root.addEventListener('pointerenter', wake);
  root.addEventListener('pointermove', wake);
  root.addEventListener('pointerleave', settle);
  settle();
  let geomKey = '';
  let selected: string | undefined;
  /** Measured length of a fit-to-content panel (0 until known). */
  let fitLen = 0;

  const editing = () => store.state.editMode;

  const origin = () => {
    const out = store.output(output);
    if (!current || !out) return { x: 0, y: 0 };
    const r = panelWindowRect(current, out, editing(), store.state.layout.panels, fitLen || undefined);
    return { x: r.x, y: r.y };
  };
  const glass = glassLayer(island, {
    output,
    origin: () => {
      const o = origin();
      const r = rectIn(root, island);
      return { x: o.x + r.x, y: o.y + r.y };
    },
  });

  // ----- fit-to-content panels -------------------------------------------
  // The dock has no fixed length: after every render the slots are measured
  // and the host is told how long the window should be (`panel.fit`).

  const measure = (): number => {
    if (!current) return 0;
    const vertical = isVertical(current);
    // everything around the widgets box: the island's insets, border and padding
    const chrome = vertical ? root.clientHeight - widgetsEl.clientHeight : root.clientWidth - widgetsEl.clientWidth;
    let lo = Infinity;
    let hi = -Infinity;
    for (const slot of Array.from(widgetsEl.children)) {
      const r = rectIn(root, slot);
      const a = vertical ? r.y : r.x;
      const b = vertical ? r.y + r.h : r.x + r.w;
      if (a < lo) lo = a;
      if (b > hi) hi = b;
    }
    return hi > lo ? Math.ceil(hi - lo) + Math.max(0, chrome) : 0;
  };
  const report = debounce(() => {
    if (!current || !isFitPanel(current) || !root.isConnected) return;
    const len = measure();
    if (!len || len === fitLen) return;
    fitLen = len;
    bridge.send('panel.fit', { length: len });
    glass.update();
  }, 30);
  const sizer = typeof ResizeObserver === 'function' ? new ResizeObserver(() => { report(); centre(); }) : undefined;

  // ----- centring ---------------------------------------------------------
  // With two expanding spacers the widgets between them are centred on the
  // bar itself, not on the space left over (the Windows way: the tray on the
  // right does not push the apps off centre). The first spacer gets a fixed
  // length, the last one takes the rest. Falls back to equal spacers when the
  // sides do not leave room.

  const centre = () => {
    if (!current || isFitPanel(current) || !root.isConnected) return;
    const vertical = isVertical(current);
    const slots = Array.from(widgetsEl.children) as HTMLElement[];
    const isExpand = (el: HTMLElement) => !!el.querySelector(':scope > .w-spacer.expand');
    const first = slots.findIndex(isExpand);
    let lastIdx = -1;
    for (let i = slots.length - 1; i > first; i--) if (isExpand(slots[i])) { lastIdx = i; break; }
    if (first < 0 || lastIdx < 0) return;
    // rectIn: unscaled pixels (the preview stage is scaled), like the flex-basis set below
    const len = (el: HTMLElement) => (vertical ? rectIn(root, el).h : rectIn(root, el).w);
    const cs = getComputedStyle(widgetsEl);
    const gap = parseFloat(cs.columnGap || cs.gap) || 0;
    const pad = vertical ? parseFloat(cs.paddingTop) + parseFloat(cs.paddingBottom) : parseFloat(cs.paddingLeft) + parseFloat(cs.paddingRight);
    const total = len(widgetsEl) - pad;
    const sum = (a: number, b: number) => slots.slice(a, b).reduce((acc, el) => acc + len(el) + gap, 0);
    const left = sum(0, first);
    const middle = sum(first + 1, lastIdx) - gap;
    const right = sum(lastIdx + 1, slots.length) - gap;
    const want = (total - middle) / 2 - left - gap;
    const rest = total - left - gap - want - middle - gap - right;
    const spacer = slots[first];
    const value = want >= 0 && rest >= 0 ? `0 0 ${Math.round(want)}px` : '';
    if (spacer.style.flex !== value) spacer.style.flex = value;
  };

  const anchorOf = (el: Element): Anchor => {
    const o = origin();
    const r = rectIn(root, el);
    return { x: o.x + r.x, y: o.y + r.y, w: r.w, h: r.h, edge: current?.edge };
  };

  const placeholder = (entry: WidgetEntry): WidgetInstance => ({
    el: h('div', { class: 'w w-unknown', title: `No widget of type "${entry.type}" is installed` }, icon('box', 16), h('span', { class: 'w-label' }, entry.type)),
  });

  const mount = (slot: HTMLElement, entry: WidgetEntry, p: PanelDef) => {
    const def = getWidget(entry.type);
    const ctx: WidgetCtx = {
      id: entry.id,
      type: entry.type,
      config: mergedConfig(def, entry.config),
      container: 'panel',
      output,
      panel: { id: p.id, edge: p.edge, size: p.size, vertical: isVertical(p) },
      store,
      origin,
      anchorOf,
      openPopup: (name, arg, opts) => actions.openPopup(name, arg, opts),
      togglePopup: (name, arg, opts) => actions.togglePopup(name, arg, opts),
      setConfig: (patch) =>
        store.updateLayout((l) => {
          const w = panelById(l, p.id)?.widgets.find((x) => x.id === entry.id);
          if (w) Object.assign(w.config, patch);
        }),
      editMode: editing,
    };
    let inst: WidgetInstance;
    try {
      inst = def ? def.create(ctx) : placeholder(entry);
    } catch (e) {
      console.error(`widget ${entry.type} failed`, e);
      inst = placeholder(entry);
    }
    slot.replaceChildren(inst.el);
    slot.dataset.type = entry.type;
    slot.addEventListener('contextmenu', (e) => widgetMenu(e, slot, entry, p));
    mounted.set(entry.id, { ctx, inst, slot });
    sizer?.observe(slot);
    if (editing()) decorate(slot, entry);
  };

  // Right-click on a widget: its settings without going through edit mode.
  // Widgets with a menu of their own (task bar items, tray icons) have
  // already handled the event, so only the bare widget gets this one.
  const widgetMenu = (e: MouseEvent, slot: HTMLElement, entry: WidgetEntry, p: PanelDef) => {
    if (e.defaultPrevented) return;
    e.preventDefault();
    const def = getWidget(entry.type);
    const target = { kind: 'panel' as const, id: p.id, widget: entry.id };
    const items: MenuAction[] = [];
    if (def?.settings && Object.keys(def.settings).length) items.push({ label: `${def.name} settings`, icon: 'gear', action: { popup: 'widget-settings', arg: { target, anchor: anchorOf(slot) } } });
    items.push(
      { label: editing() ? 'Leave edit mode' : 'Edit the panel', icon: editing() ? 'check' : 'edit', action: { editMode: !editing() } },
      { label: 'Add widget', icon: 'plus', action: { popup: 'widget-catalog', arg: { target: { kind: 'panel', id: p.id }, anchor: anchorOf(slot) } } },
      { label: '', separator: true },
      { label: `Remove ${def?.name ?? entry.type}`, icon: 'x', danger: true, action: { removeWidget: target } },
    );
    const o = origin();
    actions.openPopup('context-menu', { title: def?.name ?? entry.type, items, anchor: { x: o.x + e.clientX, y: o.y + e.clientY, w: 0, h: 0, edge: p.edge } });
  };

  const unmount = (id: string) => {
    const m = mounted.get(id);
    if (!m) return;
    try {
      m.inst.destroy?.();
    } catch (e) {
      console.error(e);
    }
    m.slot.replaceChildren();
    sizer?.unobserve(m.slot);
    mounted.delete(id);
  };

  // ----- edit mode: widget tools + drag to reorder -----------------------

  const decorate = (slot: HTMLElement, entry: WidgetEntry) => {
    if (slot.querySelector(':scope > .w-cover')) return;
    const def = getWidget(entry.type);
    const cover = h('div', { class: 'w-cover', title: 'Drag to move' });
    const tools = h(
      'div',
      { class: 'w-tools' },
      h('span', { class: 'w-tools-name' }, def?.name ?? entry.type),
      def?.settings
        ? h('button', { class: 'tool', title: 'Configure', onclick: () => actions.openPopup('widget-settings', { target: { kind: 'panel', id: panelId, widget: entry.id }, anchor: anchorOf(slot) }) }, icon('gear', 14))
        : null,
      h('button', { class: 'tool danger', title: 'Remove', onclick: () => store.updateLayout((l) => {
        const p = panelById(l, panelId);
        if (p) p.widgets = p.widgets.filter((w) => w.id !== entry.id);
      }) }, icon('x', 14)),
    );
    cover.addEventListener('pointerdown', (e) => startDrag(e, slot, entry.id));
    cover.addEventListener('click', () => {
      selected = selected === entry.id ? undefined : entry.id;
      syncSelection();
    });
    slot.append(cover, tools);
  };

  const undecorate = (slot: HTMLElement) => {
    slot.querySelectorAll(':scope > .w-cover, :scope > .w-tools').forEach((n) => n.remove());
    slot.classList.remove('selected');
  };

  const syncSelection = () => {
    for (const [id, m] of mounted) m.slot.classList.toggle('selected', id === selected);
  };

  const startDrag = (e: PointerEvent, slot: HTMLElement, id: string) => {
    if (e.button !== 0 || !current) return;
    e.preventDefault();
    const cover = e.currentTarget as HTMLElement;
    cover.setPointerCapture(e.pointerId);
    const vertical = isVertical(current);
    const startX = e.clientX;
    const startY = e.clientY;
    let dragging = false;
    let target = -1;
    const slots = () => Array.from(widgetsEl.children) as HTMLElement[];
    const move = (ev: PointerEvent) => {
      if (!dragging && Math.hypot(ev.clientX - startX, ev.clientY - startY) < 4) return;
      dragging = true;
      slot.classList.add('drag-src');
      const others = slots().filter((s) => s !== slot);
      const pos = vertical ? ev.clientY : ev.clientX;
      let idx = 0;
      for (const s of others) {
        const r = s.getBoundingClientRect();
        const mid = vertical ? r.top + r.height / 2 : r.left + r.width / 2;
        if (pos > mid) idx++;
      }
      target = idx;
      const ref = others[idx];
      const barRect = rectIn(root, bar);
      dropInd.hidden = false;
      if (ref) {
        const r = rectIn(root, ref);
        if (vertical) dropInd.style.top = `${r.y - barRect.y - 1}px`;
        else dropInd.style.left = `${r.x - barRect.x - 1}px`;
      } else {
        const lastEl = others[others.length - 1];
        const r = lastEl ? rectIn(root, lastEl) : { x: barRect.x, y: barRect.y, w: 0, h: 0 };
        if (vertical) dropInd.style.top = `${r.y + r.h - barRect.y - 1}px`;
        else dropInd.style.left = `${r.x + r.w - barRect.x - 1}px`;
      }
    };
    const up = () => {
      cover.removeEventListener('pointermove', move);
      cover.removeEventListener('pointerup', up);
      cover.removeEventListener('pointercancel', up);
      dropInd.hidden = true;
      slot.classList.remove('drag-src');
      if (!dragging || target < 0) return;
      store.updateLayout((l) => {
        const p = panelById(l, panelId);
        if (!p) return;
        const from = p.widgets.findIndex((w) => w.id === id);
        if (from < 0) return;
        const [w] = p.widgets.splice(from, 1);
        p.widgets.splice(target, 0, w);
      });
    };
    cover.addEventListener('pointermove', move);
    cover.addEventListener('pointerup', up);
    cover.addEventListener('pointercancel', up);
  };

  // ----- edit mode: the settings strip ------------------------------------

  const patch = (fn: (p: PanelDef) => void) =>
    store.updateLayout((l) => {
      const p = panelById(l, panelId);
      if (p) fn(p);
    });

  const edgeBtns = EDGES.map((e) =>
    h('button', { class: 'seg', dataset: { edge: e.edge }, title: `Move to the ${e.label.toLowerCase()} edge`, onclick: () => patch((p) => (p.edge = e.edge)) }, icon(e.icon, 14)),
  );
  const alignBtns = (['start', 'center', 'end'] as Align[]).map((a) =>
    h('button', { class: 'seg', dataset: { align: a }, onclick: () => patch((p) => (p.align = a)) }, a.toUpperCase()),
  );
  const layerBtns = (['top', 'bottom'] as PanelLayer[]).map((l) =>
    h('button', { class: 'seg', dataset: { layer: l }, title: l === 'top' ? 'Above windows' : 'Below windows', onclick: () => patch((p) => (p.layer = l)) }, l === 'top' ? 'ABOVE' : 'BELOW'),
  );
  const range = (min: number, max: number, step: number, apply: (v: number) => void) => {
    const input = h('input', { type: 'range', min, max, step }) as HTMLInputElement;
    input.addEventListener('input', () => {
      fill(input);
      apply(Number(input.value));
    });
    return input;
  };
  const fill = (i: HTMLInputElement) => {
    const lo = Number(i.min);
    const hi = Number(i.max);
    i.style.setProperty('--fill', `${((Number(i.value) - lo) / (hi - lo)) * 100}%`);
  };
  const sizeIn = range(24, 96, 2, (v) => patch((p) => (p.size = v)));
  const lengthIn = range(20, 100, 1, (v) => patch((p) => (p.length = v)));
  const fitBtns = [
    h('button', { class: 'seg', dataset: { fit: '1' }, title: 'Only as long as its widgets, centred (a dock)', onclick: () => patch((p) => (p.length = 0)) }, 'FIT'),
    h('button', { class: 'seg', dataset: { fit: '0' }, title: 'A share of the edge', onclick: () => patch((p) => { if (p.length <= 0) p.length = 100; }) }, '%'),
  ];
  const opacityIn = range(30, 100, 5, (v) => patch((p) => (p.opacity = v / 100)));
  const floatBtns = [
    h('button', { class: 'seg', dataset: { float: '1' }, title: 'Floats as a rounded island', onclick: () => patch((p) => (p.float = true)) }, 'FLOAT'),
    h('button', { class: 'seg', dataset: { float: '0' }, title: 'Flush with the screen edge', onclick: () => patch((p) => (p.float = false)) }, 'EDGE'),
  ];
  const sizeVal = h('span', { class: 'mono val' });
  const lengthVal = h('span', { class: 'mono val' });
  const opacityVal = h('span', { class: 'mono val' });
  const removeBtn = h('button', { class: 'btn danger' }, icon('x', 14), 'Remove panel');
  let armed = false;
  removeBtn.addEventListener('click', () => {
    if (!armed) {
      armed = true;
      removeBtn.classList.add('armed');
      removeBtn.lastChild!.textContent = 'Confirm removal';
      setTimeout(() => {
        armed = false;
        removeBtn.classList.remove('armed');
        removeBtn.lastChild!.textContent = 'Remove panel';
      }, 3000);
      return;
    }
    store.updateLayout((l) => (l.panels = l.panels.filter((p) => p.id !== panelId)));
  });
  const addBtn = h('button', { class: 'btn accent', onclick: () => actions.openPopup('widget-catalog', { target: { kind: 'panel', id: panelId }, anchor: anchorOf(addBtn) }) }, icon('plus', 14), 'Add widget');
  const doneBtn = h('button', { class: 'btn primary', onclick: () => store.setEditMode(false) }, icon('check', 14), 'Done');
  const field = (label: string, ...ctl: (HTMLElement | null)[]) => h('label', { class: 'field' }, h('span', { class: 'field-name' }, label), ...ctl);
  const stripBar = h(
    'div',
    { class: 'strip-bar' },
    h('div', { class: 'strip-title' }, icon('panel', 14), h('span', {}, 'PANEL')),
    field('Edge', h('span', { class: 'segs' }, ...edgeBtns)),
    field('Size', sizeIn, sizeVal),
    field('Length', h('span', { class: 'segs' }, ...fitBtns), lengthIn, lengthVal),
    field('Align', h('span', { class: 'segs' }, ...alignBtns)),
    field('Layer', h('span', { class: 'segs' }, ...layerBtns)),
    field('Style', h('span', { class: 'segs' }, ...floatBtns)),
    field('Opacity', opacityIn, opacityVal),
    h('span', { class: 'strip-gap' }),
    addBtn,
    removeBtn,
    doneBtn,
  );
  strip.append(stripBar);

  const syncStrip = (p: PanelDef) => {
    for (const b of edgeBtns) b.classList.toggle('on', b.dataset.edge === p.edge);
    for (const b of alignBtns) b.classList.toggle('on', b.dataset.align === p.align);
    for (const b of layerBtns) b.classList.toggle('on', b.dataset.layer === p.layer);
    const floats = panelFloats(p);
    for (const b of floatBtns) b.classList.toggle('on', (b.dataset.float === '1') === floats);
    const set = (i: HTMLInputElement, v: number, out: HTMLElement, text: string) => {
      if (document.activeElement !== i) i.value = String(v);
      fill(i);
      out.textContent = text;
    };
    set(sizeIn, p.size, sizeVal, `${p.size}px`);
    const fit = isFitPanel(p);
    for (const b of fitBtns) b.classList.toggle('on', (b.dataset.fit === '1') === fit);
    lengthIn.disabled = fit;
    set(lengthIn, fit ? 100 : p.length, lengthVal, fit ? 'FIT' : `${p.length}%`);
    set(opacityIn, Math.round(p.opacity * 100), opacityVal, `${Math.round(p.opacity * 100)}%`);
  };

  // ----- render -------------------------------------------------------------

  const render = () => {
    const p = panelById(store.state.layout, panelId);
    if (!p) {
      root.hidden = true;
      return;
    }
    root.hidden = false;
    current = p;
    const vertical = isVertical(p);
    const edit = editing();
    root.dataset.edge = p.edge;
    root.classList.toggle('vertical', vertical);
    root.classList.toggle('editing', edit);
    root.classList.toggle('fit', isFitPanel(p));
    root.classList.toggle('bare', p.opacity <= 0.02);
    root.classList.toggle('edge', !panelFloats(p));
    root.style.setProperty('--panel-size', `${p.size}px`);
    root.style.setProperty('--panel-opacity', String(clamp(p.opacity, 0, 1)));
    root.style.setProperty('--edit-extra', `${EDIT_EXTRA}px`);
    strip.hidden = !edit;

    const key = `${p.edge}:${p.size}`;
    if (key !== geomKey) {
      geomKey = key;
      for (const id of Array.from(mounted.keys())) unmount(id);
    }
    reconcile(
      widgetsEl,
      p.widgets,
      (w) => w.id,
      (w) => {
        const slot = h('div', { class: 'w-slot' });
        mount(slot, w, p);
        return slot;
      },
      (slot, w) => {
        const m = mounted.get(w.id);
        if (!m || m.ctx.type !== w.type) {
          unmount(w.id);
          mount(slot, w, p);
          return;
        }
        const merged = mergedConfig(getWidget(w.type), w.config);
        if (JSON.stringify(merged) !== JSON.stringify(m.ctx.config)) {
          m.ctx.config = merged;
          m.inst.update?.(merged);
        }
        if (edit) decorate(slot, w);
        else undecorate(slot);
      },
      (slot) => {
        const id = slot.dataset.key;
        if (id) unmount(id);
        slot.remove();
      },
    );
    if (edit) syncStrip(p);
    else selected = undefined;
    syncSelection();
    if (isFitPanel(p)) report();
    else fitLen = 0;
    requestAnimationFrame(() => {
      centre();
      glass.update();
    });
  };

  render();
  const offs = [store.on('layout', render), store.on('editMode', render), store.on('outputs', render)];
  return () => {
    offs.forEach((off) => off());
    sizer?.disconnect();
    glass.dispose();
    for (const id of Array.from(mounted.keys())) unmount(id);
    root.replaceChildren();
  };
}

export { allWidgets };
