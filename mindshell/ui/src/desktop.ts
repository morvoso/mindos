// The desktop window: wallpaper, desktop widgets and the edit-mode toolbar.

import * as actions from './actions';
import * as bridge from './bridge';
import { clamp, h, reconcile } from './dom';
import { EDIT_EXTRA, rectIn } from './geometry';
import { icon } from './icons';
import { desktopWidgetsForOutput, newPanel } from './layout';
import { store } from './state';
import { getWidget, mergedConfig, type WidgetCtx, type WidgetInstance } from './widgets/registry';
import type { Anchor, DesktopWidgetEntry, Edge, MenuAction } from './types';

const SNAP = 8;
const MIN_W = 120;
const MIN_H = 60;

interface Mounted {
  ctx: WidgetCtx;
  inst: WidgetInstance;
  box: HTMLElement;
}

export function renderDesktop(root: HTMLElement, output: string): () => void {
  root.classList.add('desktop-window');
  const wall = h('div', { class: 'wallpaper' });
  const mark = h('div', { class: 'wordmark' }, h('span', { class: 'wordmark-text' }, 'MINDOS'), h('span', { class: 'wordmark-sub' }, 'GAMING · DEV'));
  const layer = h('div', { class: 'desktop-widgets' });
  const toolbar = h('div', { class: 'edit-toolbar', hidden: true });
  root.append(wall, mark, layer, toolbar);

  const mounted = new Map<string, Mounted>();
  const editing = () => store.state.editMode;
  const out = () => store.output(output);

  const anchorOf = (el: Element): Anchor => {
    const r = rectIn(root, el);
    return { x: r.x, y: r.y, w: r.w, h: r.h };
  };

  // ----- wallpaper ----------------------------------------------------------

  const renderWallpaper = () => {
    const wp = store.state.layout.desktop.wallpaper;
    const image = wp.mode === 'image' && wp.path;
    wall.classList.toggle('builtin', !image);
    wall.classList.toggle('image', !!image);
    if (image) {
      const url = /^[a-z]+:\/\//i.test(wp.path!) ? wp.path! : `file://${wp.path}`;
      wall.style.backgroundImage = `url("${url}")`;
    } else wall.style.backgroundImage = '';
  };

  // ----- widgets -------------------------------------------------------------

  const patchEntry = (id: string, fn: (e: DesktopWidgetEntry) => void) =>
    store.updateLayout((l) => {
      const e = l.desktop.widgets.find((w) => w.id === id);
      if (e) fn(e);
    });

  const mount = (box: HTMLElement, entry: DesktopWidgetEntry) => {
    const def = getWidget(entry.type);
    const ctx: WidgetCtx = {
      id: entry.id,
      type: entry.type,
      config: mergedConfig(def, entry.config),
      container: 'desktop',
      output,
      store,
      origin: () => ({ x: 0, y: 0 }),
      anchorOf,
      openPopup: (name, arg, opts) => actions.openPopup(name, arg, opts),
      togglePopup: (name, arg, opts) => actions.togglePopup(name, arg, opts),
      setConfig: (patch) => patchEntry(entry.id, (e) => Object.assign(e.config, patch)),
      editMode: editing,
    };
    let inst: WidgetInstance;
    try {
      inst = def ? def.create(ctx) : { el: h('div', { class: 'dw-body dw-unknown' }, icon('box', 18), h('span', {}, `"${entry.type}" is not installed`)) };
    } catch (e) {
      console.error(`widget ${entry.type} failed`, e);
      inst = { el: h('div', { class: 'dw-body dw-unknown' }, `${entry.type} failed to load`) };
    }
    const content = h('div', { class: 'dw-content' }, inst.el);
    const cover = h('div', { class: 'dw-cover', title: 'Drag to move' });
    const chrome = h(
      'div',
      { class: 'dw-chrome' },
      h('span', { class: 'dw-grip' }, icon('grip', 12), h('span', { class: 'dw-name' }, def?.name ?? entry.type)),
      h('span', { class: 'dw-actions' },
        def?.settings ? h('button', { class: 'tool', title: 'Configure', onclick: () => actions.openPopup('widget-settings', { target: { kind: 'desktop', widget: entry.id }, anchor: anchorOf(box) }) }, icon('gear', 13)) : null,
        h('button', { class: 'tool danger', title: 'Remove', onclick: () => store.updateLayout((l) => (l.desktop.widgets = l.desktop.widgets.filter((w) => w.id !== entry.id))) }, icon('x', 13)),
      ),
    );
    const resize = h('div', { class: 'dw-resize', title: 'Resize' }, icon('resize', 12));
    box.replaceChildren(content, cover, chrome, resize);
    cover.addEventListener('pointerdown', (e) => drag(e, box, entry.id, 'move'));
    chrome.firstElementChild!.addEventListener('pointerdown', (e) => drag(e as PointerEvent, box, entry.id, 'move'));
    resize.addEventListener('pointerdown', (e) => drag(e, box, entry.id, 'resize'));
    mounted.set(entry.id, { ctx, inst, box });
  };

  const unmount = (id: string) => {
    const m = mounted.get(id);
    if (!m) return;
    try {
      m.inst.destroy?.();
    } catch (e) {
      console.error(e);
    }
    mounted.delete(id);
  };

  const place = (box: HTMLElement, e: DesktopWidgetEntry) => {
    box.style.left = `${e.x}px`;
    box.style.top = `${e.y}px`;
    box.style.width = `${e.w}px`;
    box.style.height = `${e.h}px`;
  };

  const drag = (e: PointerEvent, box: HTMLElement, id: string, mode: 'move' | 'resize') => {
    if (e.button !== 0 || !editing()) return;
    e.preventDefault();
    e.stopPropagation();
    const handle = e.currentTarget as HTMLElement;
    handle.setPointerCapture(e.pointerId);
    const scale = root.getBoundingClientRect().width / root.offsetWidth || 1;
    const o = out();
    const bounds = { w: o?.width ?? root.offsetWidth, h: o?.height ?? root.offsetHeight };
    const start = { x: e.clientX, y: e.clientY, left: box.offsetLeft, top: box.offsetTop, w: box.offsetWidth, h: box.offsetHeight };
    const snap = (v: number) => Math.round(v / SNAP) * SNAP;
    let cur = { x: start.left, y: start.top, w: start.w, h: start.h };
    let moved = false;
    const move = (ev: PointerEvent) => {
      const dx = (ev.clientX - start.x) / scale;
      const dy = (ev.clientY - start.y) / scale;
      if (!moved && Math.hypot(dx, dy) < 3) return;
      moved = true;
      box.classList.add('dragging');
      if (mode === 'move') {
        cur.x = clamp(snap(start.left + dx), 0, bounds.w - start.w);
        cur.y = clamp(snap(start.top + dy), 0, bounds.h - start.h);
      } else {
        cur.w = clamp(snap(start.w + dx), MIN_W, bounds.w - start.left);
        cur.h = clamp(snap(start.h + dy), MIN_H, bounds.h - start.top);
      }
      box.style.left = `${cur.x}px`;
      box.style.top = `${cur.y}px`;
      box.style.width = `${cur.w}px`;
      box.style.height = `${cur.h}px`;
    };
    const up = () => {
      handle.removeEventListener('pointermove', move);
      handle.removeEventListener('pointerup', up);
      handle.removeEventListener('pointercancel', up);
      box.classList.remove('dragging');
      if (!moved) return;
      patchEntry(id, (en) => Object.assign(en, cur));
    };
    handle.addEventListener('pointermove', move);
    handle.addEventListener('pointerup', up);
    handle.addEventListener('pointercancel', up);
  };

  const renderWidgets = () => {
    const entries = desktopWidgetsForOutput(store.state.layout, output);
    reconcile(
      layer,
      entries,
      (e) => e.id,
      (e) => {
        const box = h('div', { class: 'dw', dataset: { type: e.type } });
        mount(box, e);
        place(box, e);
        return box;
      },
      (box, e) => {
        const m = mounted.get(e.id);
        if (!m || m.ctx.type !== e.type) {
          unmount(e.id);
          mount(box, e);
        } else {
          const merged = mergedConfig(getWidget(e.type), e.config);
          if (JSON.stringify(merged) !== JSON.stringify(m.ctx.config)) {
            m.ctx.config = merged;
            m.inst.update?.(merged);
          }
        }
        if (!box.classList.contains('dragging')) place(box, e);
      },
      (box) => {
        const id = box.dataset.key;
        if (id) unmount(id);
      },
    );
  };

  // ----- edit toolbar ------------------------------------------------------

  const panelMenu = h('div', { class: 'menu drop', hidden: true });
  const edges: { edge: Edge; label: string; icon: string }[] = [
    { edge: 'top', label: 'Top panel', icon: 'chevron-up' },
    { edge: 'bottom', label: 'Bottom panel', icon: 'chevron-down' },
    { edge: 'left', label: 'Left panel', icon: 'chevron-left' },
    { edge: 'right', label: 'Right panel', icon: 'chevron-right' },
  ];
  for (const e of edges) {
    panelMenu.appendChild(
      h('button', { class: 'menu-item', onclick: () => {
        panelMenu.hidden = true;
        store.updateLayout((l) => {
          const p = newPanel(e.edge, l.panels);
          p.output = store.state.outputs.length > 1 ? output : '*';
          l.panels.push(p);
        });
      } }, icon(e.icon, 14), h('span', {}, e.label)),
    );
  }
  const addPanelBtn = h('button', { class: 'btn', onclick: (ev: Event) => {
    ev.stopPropagation();
    panelMenu.hidden = !panelMenu.hidden;
  } }, icon('panel', 14), 'Add panel', icon('chevron-down', 12, 'caret'));
  const resetBtn = h('button', { class: 'btn danger' }, icon('refresh', 14), 'Reset layout');
  let armed = false;
  resetBtn.addEventListener('click', () => {
    if (!armed) {
      armed = true;
      resetBtn.classList.add('armed');
      resetBtn.lastChild!.textContent = 'Confirm reset';
      setTimeout(() => {
        armed = false;
        resetBtn.classList.remove('armed');
        resetBtn.lastChild!.textContent = 'Reset layout';
      }, 3000);
      return;
    }
    bridge.call('layout.reset').catch((e) => console.warn(e));
  });
  toolbar.append(
    h('span', { class: 'edit-title' }, h('span', {}, icon('edit', 14), h('span', {}, 'EDIT MODE')), h('span', { class: 'edit-sub' }, 'Drag widgets to move or reorder · hover one for its settings')),
    h('button', { class: 'btn accent', onclick: () => actions.openPopup('widget-catalog', { target: { kind: 'desktop', output }, anchor: anchorOf(toolbar) }) }, icon('plus', 14), 'Add widget'),
    h('span', { class: 'rel' }, addPanelBtn, panelMenu),
    resetBtn,
    h('span', { class: 'edit-gap' }),
    h('button', { class: 'btn primary', onclick: () => store.setEditMode(false) }, icon('check', 14), 'Done'),
  );
  root.addEventListener('pointerdown', (e) => {
    if (!panelMenu.hidden && !panelMenu.contains(e.target as Node) && e.target !== addPanelBtn) panelMenu.hidden = true;
  });

  const placeToolbar = () => {
    // Keep clear of a top panel and its edit strip.
    let top = 24;
    for (const p of store.state.layout.panels) {
      if (p.edge !== 'top' || (p.output !== '*' && p.output !== output)) continue;
      top = Math.max(top, p.margin + p.size + EDIT_EXTRA + 16);
    }
    toolbar.style.top = `${top}px`;
  };

  // ----- context menu ------------------------------------------------------

  root.addEventListener('contextmenu', (e) => {
    if ((e.target as HTMLElement).closest('.dw, .edit-toolbar')) return;
    e.preventDefault();
    const scale = root.getBoundingClientRect().width / root.offsetWidth || 1;
    const b = root.getBoundingClientRect();
    const x = (e.clientX - b.left) / scale;
    const y = (e.clientY - b.top) / scale;
    const terminal = store.state.config.terminal || 'foot';
    const items: MenuAction[] = editing()
      ? [
          { label: 'Add widget', icon: 'plus', action: { popup: 'widget-catalog', arg: { target: { kind: 'desktop', output } } } },
          { label: 'Leave edit mode', icon: 'check', action: { editMode: false } },
        ]
      : [
          { label: 'Edit desktop', icon: 'edit', action: { editMode: true } },
          { label: 'Add widget', icon: 'plus', action: { popup: 'widget-catalog', arg: { target: { kind: 'desktop', output } } } },
          { label: '', separator: true },
          { label: 'Terminal', icon: 'terminal', action: { exec: terminal } },
          { label: 'Ask Mind', icon: 'mind', action: { call: 'mind.toggle' } },
          { label: 'Files', icon: 'folder', action: { call: 'shell.openApp', params: { name: 'files' } } },
          { label: '', separator: true },
          { label: 'Change wallpaper', icon: 'image', action: { call: 'shell.openApp', params: { name: 'settings', page: 'wallpaper' } } },
          { label: 'Display settings', icon: 'display', action: { call: 'shell.openApp', params: { name: 'settings', page: 'displays' } } },
          { label: 'Settings', icon: 'gear', action: { call: 'shell.openApp', params: { name: 'settings' } } },
        ];
    actions.openPopup('context-menu', { items, anchor: { x, y, w: 0, h: 0 } });
  });

  // ----- render -------------------------------------------------------------

  const render = () => {
    const edit = editing();
    root.classList.toggle('editing', edit);
    toolbar.hidden = !edit;
    panelMenu.hidden = true;
    renderWallpaper();
    renderWidgets();
    if (edit) placeToolbar();
  };
  render();
  const offs = [store.on('layout', render), store.on('editMode', render), store.on('outputs', render)];
  return () => {
    offs.forEach((off) => off());
    for (const id of Array.from(mounted.keys())) unmount(id);
    root.replaceChildren();
  };
}
