// Browser preview: one 1920×1080 output with the desktop, its panels and any
// popups laid out at the geometry the host would use. Used for screenshots
// and for developing without the compositor.

import { renderApp } from './apps';
import * as bridge from './bridge';
import { renderDesktop } from './desktop';
import { h } from './dom';
import { panelWindowRect } from './geometry';
import { newPanel, newWidget, panelsForOutput } from './layout';
import { mockHooks } from './mock';
import { renderPanel } from './panel';
import { renderPopupWindow } from './popups';
import { store } from './state';

export function renderPreview(root: HTMLElement): void {
  const params = new URLSearchParams(location.search);
  document.body.classList.add('preview');
  const out = store.state.outputs[0] ?? { name: 'preview', x: 0, y: 0, width: 1920, height: 1080, scale: 1 };
  const stage = h('div', { class: 'stage' });
  stage.style.width = `${out.width}px`;
  stage.style.height = `${out.height}px`;
  root.appendChild(stage);

  const desk = h('div', { class: 'win win-desktop' });
  stage.appendChild(desk);
  renderDesktop(desk, out.name);

  const panelWins = new Map<string, { box: HTMLElement; dispose: () => void }>();
  const fitLens = new Map<string, number>();
  const layoutPanels = () => {
    const panels = panelsForOutput(store.state.layout, out.name);
    const seen = new Set<string>();
    for (const p of panels) {
      seen.add(p.id);
      let w = panelWins.get(p.id);
      if (!w) {
        const box = h('div', { class: 'win win-panel', dataset: { panel: p.id } });
        stage.appendChild(box);
        w = { box, dispose: renderPanel(box, p.id, out.name) };
        panelWins.set(p.id, w);
      }
      const r = panelWindowRect(p, out, store.state.editMode, store.state.layout.panels, fitLens.get(p.id));
      w.box.style.left = `${r.x}px`;
      w.box.style.top = `${r.y}px`;
      w.box.style.width = `${r.w}px`;
      w.box.style.height = `${r.h}px`;
      w.box.style.zIndex = p.layer === 'top' ? '20' : '5';
    }
    for (const [id, w] of panelWins) {
      if (!seen.has(id)) {
        w.dispose();
        w.box.remove();
        panelWins.delete(id);
      }
    }
  };
  layoutPanels();
  store.on('layout', layoutPanels);
  store.on('editMode', layoutPanels);
  mockHooks.panelFit = (id, length) => {
    fitLens.set(id, length);
    layoutPanels();
  };
  // App windows open as floating boxes on the stage, roughly where the compositor would put them.
  let appCount = 0;
  mockHooks.openApp = (name, page, arg) => {
    const box = h('div', { class: 'win win-app', dataset: { app: name } });
    const w = 1040;
    const hh = 700;
    box.style.width = `${w}px`;
    box.style.height = `${hh}px`;
    box.style.left = `${Math.round((out.width - w) / 2) + appCount * 32}px`;
    box.style.top = `${Math.round((out.height - hh) / 2) + appCount * 32}px`;
    appCount++;
    stage.appendChild(box);
    renderApp(box, name, { page, arg });
  };

  const popups = new Map<string, { box: HTMLElement; dispose: () => void }>();
  const closePopup = (name: string) => {
    const p = popups.get(name);
    if (!p) return;
    p.dispose();
    p.box.remove();
    popups.delete(name);
  };
  mockHooks.openPopup = (name, arg) => {
    closePopup(name);
    const box = h('div', { class: 'win win-popup', dataset: { popup: name } });
    stage.appendChild(box);
    const dispose = renderPopupWindow(box, name, arg, out.name, () => bridge.send('popup.close', { name }));
    popups.set(name, { box, dispose });
  };
  mockHooks.closePopup = closePopup;

  const fit = () => {
    const s = Math.min(window.innerWidth / out.width, window.innerHeight / out.height);
    stage.style.transform = `scale(${s})`;
    stage.style.left = `${Math.round((window.innerWidth - out.width * s) / 2)}px`;
    stage.style.top = `${Math.round((window.innerHeight - out.height * s) / 2)}px`;
  };
  fit();
  window.addEventListener('resize', fit);

  // Open the popups named in the URL the way a user would: through the widget.
  const wanted = (params.get('popup') ?? '').split(',').map((s) => s.trim()).filter(Boolean);
  const openWanted = () => {
    for (const name of wanted) {
      const click = (sel: string) => (stage.querySelector(sel) as HTMLElement | null)?.click();
      const context = (sel: string) => {
        const el = stage.querySelector(sel) as HTMLElement | null;
        if (!el) return;
        const r = el.getBoundingClientRect();
        el.dispatchEvent(new MouseEvent('contextmenu', { bubbles: true, cancelable: true, clientX: r.left + r.width / 2, clientY: r.top + r.height / 2 }));
      };
      switch (name) {
        case 'layout-mode':
          click('.w-layout-mode');
          break;
        case 'calendar':
          click('.w-clock');
          break;
        case 'audio':
          click('.w-audio');
          break;
        case 'power':
          click('.w-power');
          break;
        case 'tray-menu':
          context('.tray-item');
          break;
        case 'context-menu': {
          const el = desk;
          const r = el.getBoundingClientRect();
          el.dispatchEvent(new MouseEvent('contextmenu', { bubbles: true, cancelable: true, clientX: r.left + 700, clientY: r.top + 520 }));
          break;
        }
        case 'widget-menu':
          context('.w-slot[data-type=clock]');
          break;
        case 'widget-catalog':
          bridge.send('popup.open', { name, arg: { target: { kind: 'desktop', output: out.name } } });
          break;
        case 'widget-settings': {
          const type = params.get('widget') ?? 'clock';
          const dw = store.state.layout.desktop.widgets.find((x) => x.type === type);
          if (dw) {
            bridge.send('popup.open', { name, arg: { target: { kind: 'desktop', widget: dw.id } } });
            break;
          }
          const p = store.state.layout.panels.find((x) => x.widgets.some((w) => w.type === type));
          const w = p?.widgets.find((x) => x.type === type);
          const slot = stage.querySelector(`.w-slot[data-type="${type}"]`);
          const r = slot?.getBoundingClientRect();
          const sr = stage.getBoundingClientRect();
          const sc = sr.width / stage.offsetWidth || 1;
          const anchor = r ? { x: (r.left - sr.left) / sc, y: (r.top - sr.top) / sc, w: r.width / sc, h: r.height / sc, edge: p?.edge } : undefined;
          if (p && w) bridge.send('popup.open', { name, arg: { target: { kind: 'panel', id: p.id, widget: w.id }, anchor } });
          break;
        }
        default:
          bridge.send('popup.open', { name, arg: {} });
      }
    }
  };
  if (wanted.length) setTimeout(openWanted, 60);
  if (params.get('vertical') === '1') {
    void store.updateLayout((l) => {
      const p = newPanel('left', l.panels);
      p.size = 56;
      p.widgets = [newWidget('taskbar', { pins: ['firefox.desktop', 'steam.desktop'] }), newWidget('spacer', { expand: true }), newWidget('sysmon'), newWidget('layout-mode'), newWidget('clock')];
      l.panels.push(p);
    });
  }
  if (params.get('dwidgets') === '1') {
    void store.updateLayout((l) => {
      l.desktop.widgets.push(
        { ...newWidget('desktop-clock', { hour24: false, seconds: false, date: true }), output: out.name, x: 1400, y: 120, w: 420, h: 130 },
        { ...newWidget('desktop-notes', { title: 'TODO', text: 'Flash the ISO\nTry the columns layout' }), output: out.name, x: 1500, y: 300, w: 320, h: 220 },
      );
    });
  }
  if (params.get('stack') === '1') {
    void store.updateLayout((l) => {
      for (const p of l.panels) for (const w of p.widgets) if (w.type === 'clock') Object.assign(w.config, { stack: true, dateFormat: 'numeric' });
    });
  }
  if (params.get('labels') === '1') {
    void store.updateLayout((l) => {
      for (const p of l.panels) for (const w of p.widgets) if (w.type === 'taskbar') w.config.labels = true;
    });
  }
  const app = params.get('app');
  if (app) setTimeout(() => mockHooks.openApp?.(app, params.get('page') ?? undefined, params.get('arg') ?? undefined), 40);
  if (params.get('demo') === '1') {
    setTimeout(() => {
      stage.querySelector('.w-slot[data-type=taskbar]')?.classList.add('selected');
      stage.querySelector('.dw')?.classList.add('hover');
    }, 80);
  }
}
