import * as bridge from '../bridge';
import { h, reconcile } from '../dom';
import { hashHue, iconSvg, letterIcon } from '../icons';
import { registerWidget } from './registry';
import { outputPoint } from './common';
import type { AppInfo, MenuAction, ShellState, WindowInfo } from '../types';

interface Group {
  key: string;
  app?: AppInfo;
  label: string;
  windows: WindowInfo[];
  pinned: boolean;
  /** A Windows program (Wine/Proton): the icon carries a badge saying so. */
  wine: boolean;
}

const norm = (s: string) => s.toLowerCase().replace(/\.desktop$/, '');
const last = (s: string) => {
  const parts = s.split('.');
  return parts[parts.length - 1];
};
const execBase = (exec: string) => {
  const first = exec.trim().split(/\s+/)[0] ?? '';
  return first.split('/').pop() ?? '';
};

/** Index apps by the names a window's app_id is likely to carry. */
function appIndex(apps: AppInfo[]): Map<string, AppInfo> {
  const idx = new Map<string, AppInfo>();
  const put = (k: string, a: AppInfo) => {
    if (k && !idx.has(k)) idx.set(k, a);
  };
  // StartupWMClass is the entry's own statement of what its windows are
  // called, so it wins over the guesses below (Wine's generated entries rely
  // on it: the id is a menu path, the windows carry the exe name).
  for (const a of apps) if (a.wmClass) put(norm(a.wmClass), a);
  for (const a of apps) if (a.wmClass) put(norm(a.wmClass).replace(/\.exe$/, ''), a);
  for (const a of apps) put(norm(a.id), a);
  for (const a of apps) put(last(norm(a.id)), a);
  for (const a of apps) put(norm(execBase(a.exec)), a);
  for (const a of apps) put(norm(a.name), a);
  return idx;
}

function matchApp(idx: Map<string, AppInfo>, appId: string): AppInfo | undefined {
  const k = norm(appId);
  return idx.get(k) ?? idx.get(last(k)) ?? idx.get(k.replace(/-bin$|-wayland$|\.exe$/, ''));
}

function buildGroups(state: ShellState, pins: string[], windows: WindowInfo[]): Group[] {
  const idx = appIndex(state.apps);
  const groups: Group[] = [];
  const byApp = new Map<AppInfo, Group>();
  const byKey = new Map<string, Group>();
  for (const pin of pins) {
    const app = state.apps.find((a) => a.id === pin);
    const g: Group = { key: `pin:${pin}`, app, label: app?.name ?? norm(pin), windows: [], pinned: true, wine: !!app?.wine };
    groups.push(g);
    if (app) byApp.set(app, g);
    else byKey.set(norm(pin), g);
  }
  for (const w of windows) {
    const app = matchApp(idx, w.app_id);
    let g = app ? byApp.get(app) : byKey.get(norm(w.app_id));
    if (!g) {
      g = { key: app ? `app:${app.id}` : `win:${norm(w.app_id) || w.id}`, app, label: app?.name ?? (w.app_id || w.title || 'Window'), windows: [], pinned: false, wine: !!app?.wine };
      groups.push(g);
      if (app) byApp.set(app, g);
      else byKey.set(norm(w.app_id), g);
    }
    g.windows.push(w);
    if (w.wine) g.wine = true;
  }
  return groups;
}

registerWidget({
  type: 'taskbar',
  name: 'Task bar',
  description: 'Pinned applications and open windows. Click to focus (a second click minimises in floating mode), middle-click for a new window, right-click for more.',
  icon: 'window',
  containers: ['panel'],
  defaults: { pins: [], labels: false, maxLabel: 160, showRunning: true, onlyThisOutput: false, indicator: true },
  settings: {
    pins: { label: 'Pinned applications', type: 'list', help: 'Desktop entry IDs, one per line. Right-click a running application to pin it.', placeholder: 'firefox.desktop' },
    showRunning: { label: 'Show open windows', type: 'boolean', help: 'When off, only pinned applications are shown' },
    onlyThisOutput: { label: 'Only windows on this display', type: 'boolean', when: (c) => !!c.showRunning },
    labels: { label: 'Show window titles', type: 'boolean', when: (c) => !!c.showRunning },
    maxLabel: { label: 'Title width', type: 'number', min: 80, max: 320, step: 10, unit: 'px', when: (c) => !!c.showRunning && !!c.labels },
    indicator: { label: 'Running indicator', type: 'boolean', help: 'A dot under each running application' },
  },
  create(ctx) {
    const el = h('div', { class: 'w w-taskbar' });
    let cfg = ctx.config;

    const iconFor = (g: Group) => g.app?.icon ?? letterIcon(g.label, hashHue(g.label));

    const onClick = (g: Group, e: MouseEvent) => {
      if (e.button === 1) {
        if (g.app) bridge.send('apps.launch', { id: g.app.id });
        return;
      }
      if (g.windows.length === 0) {
        if (g.app) bridge.send('apps.launch', { id: g.app.id });
        return;
      }
      if (g.windows.length === 1) {
        const w = g.windows[0];
        // Floating mode: a second click on the focused app puts it away. In the
        // tiling modes tiles are never minimised; the compositor brings the
        // window into view (the columns strip slides over to it).
        const tiling = !!ctx.store.layoutMode && ctx.store.layoutMode.mode !== 'floating';
        if (w.focused && !w.minimized && !tiling) bridge.send('windows.minimize', { id: w.id });
        else bridge.send('windows.focus', { id: w.id });
        return;
      }
      const i = g.windows.findIndex((w) => w.focused);
      const next = g.windows[(i + 1) % g.windows.length];
      bridge.send('windows.focus', { id: next.id });
    };

    const onContext = (g: Group, e: MouseEvent) => {
      e.preventDefault();
      const items: MenuAction[] = [];
      if (g.app) items.push({ label: `New ${g.app.name} window`, icon: 'plus', action: { call: 'apps.launch', params: { id: g.app.id } } });
      if (g.windows.length > 1) {
        items.push({ label: '', separator: true });
        for (const w of g.windows) items.push({ label: w.title || g.label, icon: 'window', action: { call: 'windows.focus', params: { id: w.id } } });
      }
      items.push({ label: '', separator: true });
      if (g.app && ctx.panel) {
        items.push({
          label: g.pinned ? 'Unpin from task bar' : 'Pin to task bar',
          icon: 'pin',
          action: { pin: { panel: ctx.panel.id, widget: ctx.id, app: g.app.id, pinned: !g.pinned } },
        });
      }
      if (g.windows.length === 1) items.push({ label: 'Close', icon: 'x', danger: true, action: { call: 'windows.close', params: { id: g.windows[0].id } } });
      else if (g.windows.length > 1) {
        for (const w of g.windows) items.push({ label: `Close ${w.title || g.label}`, icon: 'x', danger: true, action: { call: 'windows.close', params: { id: w.id } } });
      }
      const p = outputPoint(ctx, e);
      ctx.openPopup('context-menu', { title: g.label, items, anchor: { x: p.x, y: p.y, w: 0, h: 0, edge: ctx.panel?.edge } });
    };

    const render = () => {
      const state = ctx.store.state;
      const pins = Array.isArray(cfg.pins) ? (cfg.pins as string[]) : [];
      const windows = state.windows.filter((w) => !cfg.onlyThisOutput || !w.output || w.output === ctx.output);
      let groups = buildGroups(state, pins, windows);
      if (cfg.showRunning === false) groups = groups.filter((g) => g.pinned);
      el.classList.toggle('labels', !!cfg.labels && cfg.showRunning !== false);
      el.classList.toggle('no-indicator', cfg.indicator === false);
      reconcile(
        el,
        groups,
        (g) => g.key,
        (g) => {
          const img = h('img', { class: 'task-ic', src: iconFor(g), alt: '', draggable: false });
          const badge = h('span', { class: 'task-badge', title: 'Windows application (Wine)' });
          badge.innerHTML = iconSvg('winapp', 9);
          const icon = h('span', { class: 'task-ic-wrap' }, img, badge);
          const label = h('span', { class: 'task-label' });
          const dots = h('span', { class: 'task-dots' });
          const item = h('div', { class: 'task', tabindex: -1 }, icon, label, dots);
          item.addEventListener('click', (e) => onClick(current.get(g.key) ?? g, e));
          item.addEventListener('auxclick', (e) => {
            if (e.button === 1) onClick(current.get(g.key) ?? g, e);
          });
          item.addEventListener('contextmenu', (e) => onContext(current.get(g.key) ?? g, e));
          return item;
        },
        (item, g) => {
          const img = item.firstElementChild!.firstElementChild as HTMLImageElement;
          const src = iconFor(g);
          if (img.getAttribute('src') !== src) img.src = src;
          const label = item.children[1] as HTMLElement;
          const focusedWin = g.windows.find((w) => w.focused);
          const text = cfg.labels ? (focusedWin ?? g.windows[0])?.title || g.label : '';
          if (label.textContent !== text) label.textContent = text;
          label.style.maxWidth = `${Number(cfg.maxLabel) || 160}px`;
          const titles = g.windows.length ? g.windows.map((w) => w.title).join('\n') : g.label;
          item.title = g.wine ? `${titles}\nWindows application (Wine)` : titles;
          item.classList.toggle('wine', g.wine);
          item.classList.toggle('running', g.windows.length > 0);
          item.classList.toggle('active', g.windows.some((w) => w.focused));
          item.classList.toggle('minimized', g.windows.length > 0 && g.windows.every((w) => w.minimized));
          item.classList.toggle('pinned', g.pinned);
          const dots = item.children[2] as HTMLElement;
          const n = Math.min(3, g.windows.length);
          if (dots.childElementCount !== n) {
            dots.textContent = '';
            for (let i = 0; i < n; i++) dots.appendChild(h('i'));
          }
        },
      );
      current = new Map(groups.map((g) => [g.key, g]));
    };
    let current = new Map<string, Group>();
    render();
    ctx.store.bind(el, 'windows', render);
    ctx.store.bind(el, 'apps', render);
    if (!ctx.store.layoutMode) void ctx.store.fetchLayoutMode();
    return {
      el,
      update(c) {
        cfg = c;
        render();
      },
    };
  },
});
