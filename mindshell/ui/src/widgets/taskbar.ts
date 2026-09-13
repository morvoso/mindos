// The task bar: pinned launchers, then the windows that are actually open,
// grouped by application. Hovering a group opens a card listing its windows
// (the panel's flyout, drawn inside the panel window) so a single window can
// be picked out of a group of eight without cycling through them.

import * as bridge from '../bridge';
import { h, reconcile } from '../dom';
import { hashHue, icon, iconSvg, letterIcon } from '../icons';
import { registerWidget } from './registry';
import { outputPoint } from './common';
import { appIndex, launchWithFeedback, matchApp, norm } from '../app-match';
import type { AppInfo, MenuAction, ShellState, WindowInfo } from '../types';

/** Where a window comes from. Wine wins: a Windows program under X11 is still
 *  a Windows program as far as the person looking at the bar is concerned. */
type Origin = 'wine' | 'x11' | 'wayland';

const originOf = (w: WindowInfo): Origin => (w.wine ? 'wine' : w.x11 ? 'x11' : 'wayland');
const ORIGIN_ICON: Record<Origin, string> = { wine: 'winapp', x11: 'x11', wayland: 'linux' };
const ORIGIN_NAME: Record<Origin, string> = { wine: 'Windows program (Wine)', x11: 'X11 (Xwayland)', wayland: 'Wayland (native)' };
const ORIGIN_SHORT: Record<Origin, string> = { wine: 'Windows', x11: 'X11', wayland: 'Wayland' };

interface Group {
  key: string;
  app?: AppInfo;
  label: string;
  windows: WindowInfo[];
  /** A pinned shortcut slot: it is on the bar whether or not anything runs. */
  pinned: boolean;
  /** In the split layout a pin knows its application is running to its right. */
  elsewhere: number;
  /** The strongest origin among its windows (a pin with none: from the entry). */
  origin: Origin;
}

const groupOrigin = (windows: WindowInfo[], app?: AppInfo): Origin => {
  if (windows.some((w) => w.wine) || (!windows.length && app?.wine)) return 'wine';
  if (windows.length && windows.every((w) => w.x11)) return 'x11';
  return 'wayland';
};

/**
 * The bar's two halves. `merge` folds the running windows back into their
 * pinned shortcut (the old single-row task bar); by default the shortcuts
 * stay a fixed row of launchers and the open windows are listed after them,
 * so a pinned application never moves as its windows come and go.
 */
function buildGroups(state: ShellState, pins: string[], windows: WindowInfo[], merge: boolean): { pinned: Group[]; running: Group[] } {
  const idx = appIndex(state.apps);
  const pinApps = new Map<string, AppInfo | undefined>();
  const pinGroups: Group[] = [];
  for (const pin of pins) {
    const app = state.apps.find((a) => a.id === pin);
    pinApps.set(pin, app);
    pinGroups.push({ key: `pin:${pin}`, app, label: app?.name ?? norm(pin), windows: [], pinned: true, elsewhere: 0, origin: app?.wine ? 'wine' : 'wayland' });
  }
  const byApp = new Map<AppInfo, Group>();
  const byKey = new Map<string, Group>();
  if (merge) {
    for (const g of pinGroups) {
      if (g.app) byApp.set(g.app, g);
      else byKey.set(norm(g.key.slice(4)), g);
    }
  }
  const running: Group[] = [];
  for (const w of windows) {
    const app = matchApp(idx, w.app_id);
    let g = app ? byApp.get(app) : byKey.get(norm(w.app_id));
    if (!g) {
      g = {
        key: app ? `app:${app.id}` : `win:${norm(w.app_id) || w.id}`,
        app,
        label: app?.name ?? (w.app_id || w.title || 'Window'),
        windows: [],
        pinned: false,
        elsewhere: 0,
        origin: 'wayland',
      };
      running.push(g);
      if (app) byApp.set(app, g);
      else byKey.set(norm(w.app_id), g);
    }
    g.windows.push(w);
  }
  for (const g of [...pinGroups, ...running]) g.origin = groupOrigin(g.windows, g.app);
  if (!merge) {
    // A shortcut whose application is open says so quietly, so the pin and the
    // window group on the other side of the separator read as one thing.
    for (const g of pinGroups) {
      const app = pinApps.get(g.key.slice(4));
      const run = running.find((r) => (app ? r.app === app : norm(r.key.slice(4)) === norm(g.key.slice(4))));
      g.elsewhere = run?.windows.length ?? 0;
    }
  }
  return { pinned: pinGroups, running: merge ? [] : running };
}

const winState = (w: WindowInfo): string =>
  w.minimized ? 'Minimised' : w.fullscreen ? 'Full screen' : w.maximized ? 'Maximised' : w.focused ? 'Focused' : 'Open';

registerWidget({
  type: 'taskbar',
  name: 'Task bar',
  description: 'Pinned shortcuts, then the open windows grouped by application. Hover a group for its window list, click to focus, middle-click for a new window, right-click for more.',
  icon: 'window',
  containers: ['panel'],
  defaults: {
    pins: [],
    labels: false,
    maxLabel: 160,
    showRunning: true,
    onlyThisOutput: true,
    indicator: true,
    mergePinned: false,
    separator: true,
    preview: true,
    previewDelay: 320,
    osBadge: 'foreign',
    iconSize: 0,
  },
  settings: {
    pins: { label: 'Pinned applications', type: 'list', help: 'Desktop entry IDs, one per line. Right-click a running application to pin it.', placeholder: 'firefox.desktop' },
    showRunning: { label: 'Show open windows', type: 'boolean', help: 'The group of open windows to the right of the shortcuts' },
    mergePinned: { label: 'Fold windows into their shortcut', type: 'boolean', help: 'One row, as before: a pinned application shows its own windows instead of appearing twice', when: (c) => !!c.showRunning },
    separator: { label: 'Separator after the shortcuts', type: 'boolean', when: (c) => !!c.showRunning && !c.mergePinned },
    onlyThisOutput: { label: 'Only windows on this display', type: 'boolean', when: (c) => !!c.showRunning },
    preview: { label: 'Window list on hover', type: 'boolean', help: 'A card beside the group listing every window, with its state', when: (c) => !!c.showRunning },
    previewDelay: { label: 'Hover delay', type: 'number', min: 0, max: 1200, step: 20, unit: 'ms', when: (c) => !!c.showRunning && c.preview !== false },
    labels: { label: 'Show window titles', type: 'boolean', when: (c) => !!c.showRunning },
    maxLabel: { label: 'Title width', type: 'number', min: 80, max: 320, step: 10, unit: 'px', when: (c) => !!c.showRunning && !!c.labels },
    indicator: { label: 'Window indicator', type: 'boolean', help: 'A mark under each icon, one per window; hollow when the window is minimised' },
    osBadge: {
      label: 'Platform badge',
      type: 'enum',
      segmented: true,
      options: [
        { value: 'off', label: 'Off' },
        { value: 'foreign', label: 'Foreign' },
        { value: 'all', label: 'All' },
      ],
      help: 'A corner tag saying where a window comes from. Foreign marks Wine and X11 only; the window list always says.',
    },
    iconSize: { label: 'Icon size', type: 'number', min: 0, max: 64, step: 2, unit: 'px', help: '0 follows the panel: a taller bar gets bigger icons' },
  },
  create(ctx) {
    const el = h('div', { class: 'w w-taskbar' });
    const pinsEl = h('div', { class: 'task-pins' });
    const sepEl = h('div', { class: 'task-sep', hidden: true });
    const runEl = h('div', { class: 'task-run' });
    const emptyEl = h('div', { class: 'task-none', hidden: true }, 'No open windows');
    el.append(pinsEl, sepEl, runEl, emptyEl);
    let cfg = ctx.config;
    let current = new Map<string, Group>();

    const iconFor = (g: Group) => g.app?.icon ?? letterIcon(g.label, hashHue(g.label));
    const tiling = () => !!ctx.store.layoutMode && ctx.store.layoutMode.mode !== 'floating';

    // ----- the window list card -------------------------------------------
    // One card, refilled for whichever group is being pointed at: the panel
    // owns where it goes, the widget owns what is in it.

    const cardIc = h('img', { class: 'fly-ic', alt: '', draggable: false });
    const cardTitle = h('div', { class: 'fly-title' });
    const cardMeta = h('div', { class: 'fly-meta' });
    const cardRows = h('div', { class: 'fly-rows' });
    const cardActions = h('div', { class: 'fly-actions' });
    const cardEl = h(
      'div',
      { class: 'fly' },
      h('div', { class: 'fly-head' }, cardIc, h('div', { class: 'fly-head-txt' }, cardTitle, cardMeta)),
      cardRows,
      cardActions,
    );
    let cardKey: string | undefined;
    let hoverTimer: ReturnType<typeof setTimeout> | undefined;

    const focusWindow = (w: WindowInfo) => {
      if (w.minimized) bridge.send('windows.unminimize', { id: w.id });
      bridge.send('windows.focus', { id: w.id });
    };

    const action = (label: string, glyph: string, run: () => void, danger = false) => {
      const b = h('button', { class: `fly-act${danger ? ' danger' : ''}`, type: 'button' }, icon(glyph, 14), h('span', {}, label));
      b.addEventListener('click', run);
      return b;
    };

    const fillCard = (g: Group) => {
      const src = iconFor(g);
      if (cardIc.getAttribute('src') !== src) cardIc.src = src;
      cardTitle.textContent = g.label;
      const n = g.windows.length;
      cardMeta.replaceChildren(
        h('span', { class: `fly-origin ${g.origin}`, title: ORIGIN_NAME[g.origin] }, icon(ORIGIN_ICON[g.origin], 12), h('span', {}, ORIGIN_SHORT[g.origin])),
        h('span', { class: 'fly-count' }, n === 1 ? '1 window' : `${n} windows`),
      );
      const many = ctx.store.state.outputs.length > 1;
      reconcile(
        cardRows,
        g.windows,
        (w) => String(w.id),
        (w) => {
          const row = h(
            'div',
            { class: 'fly-row', tabindex: 0, role: 'button' },
            h('span', { class: 'fly-row-ic' }),
            h('span', { class: 'fly-row-txt' }, h('span', { class: 'fly-row-title' }), h('span', { class: 'fly-row-sub' })),
            h('button', { class: 'fly-close', type: 'button', title: 'Close window', 'aria-label': 'Close window' }, icon('x', 13)),
          );
          const live = () => ctx.store.state.windows.find((x) => x.id === w.id) ?? w;
          row.addEventListener('click', (e) => {
            if ((e.target as Element).closest('.fly-close')) return;
            focusWindow(live());
            ctx.flyout?.hide();
          });
          row.addEventListener('keydown', (e) => {
            if (e.key !== 'Enter' && e.key !== ' ') return;
            e.preventDefault();
            focusWindow(live());
            ctx.flyout?.hide();
          });
          row.addEventListener('auxclick', (e) => {
            if (e.button === 1) bridge.send('windows.close', { id: w.id });
          });
          row.querySelector('.fly-close')!.addEventListener('click', (e) => {
            e.stopPropagation();
            bridge.send('windows.close', { id: w.id });
          });
          return row;
        },
        (row, w) => {
          const glyph = ORIGIN_ICON[originOf(w)];
          const ic = row.firstElementChild as HTMLElement;
          if (ic.dataset.g !== glyph) {
            ic.dataset.g = glyph;
            ic.innerHTML = iconSvg(glyph, 15);
          }
          ic.title = ORIGIN_NAME[originOf(w)];
          const title = row.querySelector('.fly-row-title')!;
          const text = w.title || g.label;
          if (title.textContent !== text) title.textContent = text;
          const bits = [winState(w)];
          if (many && w.output) bits.push(w.output);
          const sub = row.querySelector('.fly-row-sub')!;
          const subText = bits.join(' · ');
          if (sub.textContent !== subText) sub.textContent = subText;
          row.classList.toggle('active', w.focused && !w.minimized);
          row.classList.toggle('minimized', w.minimized);
          row.title = `${w.title || g.label}\n${bits.join(' · ')}`;
        },
      );
      const acts: HTMLElement[] = [];
      if (g.app) acts.push(action('New window', 'plus', () => bridge.send('apps.launch', { id: g.app!.id })));
      if (n > 1 || g.windows.some((w) => w.minimized)) {
        const allMin = n > 0 && g.windows.every((w) => w.minimized);
        acts.push(
          action(allMin ? 'Restore all' : 'Minimise all', allMin ? 'restore' : 'minimize', () => {
            for (const w of g.windows) bridge.send(allMin ? 'windows.unminimize' : 'windows.minimize', { id: w.id });
          }),
        );
      }
      if (n > 0) {
        acts.push(
          action(n > 1 ? 'Close all' : 'Close', 'x', () => {
            for (const w of g.windows) bridge.send('windows.close', { id: w.id });
            ctx.flyout?.hide();
          }, true),
        );
      }
      cardActions.replaceChildren(...acts);
    };

    const flyKey = (g: Group) => `${ctx.id}:${g.key}`;

    const showCard = (g: Group, anchor: Element) => {
      if (!ctx.flyout || cfg.preview === false || !g.windows.length) return;
      fillCard(g);
      cardKey = g.key;
      ctx.flyout.show(flyKey(g), cardEl, anchor);
    };

    const armCard = (g: Group, anchor: Element) => {
      if (!ctx.flyout || cfg.preview === false || !g.windows.length || ctx.editMode()) return;
      if (hoverTimer) clearTimeout(hoverTimer);
      // Once a card is up, moving along the bar switches it at once: the
      // delay is there to keep a card from appearing on the way past.
      const set = Number(cfg.previewDelay);
      const wait = ctx.flyout.shown() ? 0 : Number.isFinite(set) ? Math.max(0, set) : 320;
      hoverTimer = setTimeout(() => showCard(current.get(g.key) ?? g, anchor), wait);
    };

    const disarm = () => {
      if (hoverTimer) clearTimeout(hoverTimer);
      hoverTimer = undefined;
    };

    /** Keep the open card honest as windows come and go. */
    const refreshCard = () => {
      if (!ctx.flyout || !cardKey) return;
      const g = current.get(cardKey);
      if (!g || !g.windows.length) {
        ctx.flyout.hide(cardKey ? `${ctx.id}:${cardKey}` : undefined);
        cardKey = undefined;
        return;
      }
      if (ctx.flyout.shown() === flyKey(g)) fillCard(g);
      else cardKey = undefined;
    };

    // ----- the items on the bar -------------------------------------------

    const onClick = (g: Group, e: MouseEvent) => {
      disarm();
      if (e.button === 1) {
        if (g.app) launchWithFeedback(e.currentTarget as HTMLElement, g.app.id).catch(() => {});
        return;
      }
      if (g.windows.length === 0) {
        if (g.app) launchWithFeedback(e.currentTarget as HTMLElement, g.app.id).catch(() => {});
        return;
      }
      if (g.windows.length === 1) {
        const w = g.windows[0];
        // Floating mode: a second click on the focused app puts it away. In the
        // tiling modes tiles are never minimised; the compositor brings the
        // window into view (the columns strip slides over to it).
        if (w.focused && !w.minimized && !tiling()) bridge.send('windows.minimize', { id: w.id });
        else focusWindow(w);
        ctx.flyout?.hide();
        return;
      }
      // A group of windows: the card is the precise way in, so a click just
      // brings it up rather than guessing which window was meant.
      if (ctx.flyout && cfg.preview !== false && !ctx.editMode()) {
        if (ctx.flyout.shown() === flyKey(g)) ctx.flyout.hide();
        else showCard(g, e.currentTarget as Element);
        return;
      }
      const i = g.windows.findIndex((w) => w.focused);
      focusWindow(g.windows[(i + 1) % g.windows.length]);
    };

    const onWheel = (g: Group, e: WheelEvent) => {
      if (g.windows.length < 2) return;
      e.preventDefault();
      const d = Math.sign(Math.abs(e.deltaX) > Math.abs(e.deltaY) ? e.deltaX : e.deltaY) || 1;
      const i = g.windows.findIndex((w) => w.focused);
      const n = g.windows.length;
      focusWindow(g.windows[((i < 0 ? 0 : i + d) + n) % n]);
    };

    const onContext = (g: Group, e: MouseEvent) => {
      e.preventDefault();
      disarm();
      const items: MenuAction[] = [];
      if (g.app) items.push({ label: `New ${g.app.name} window`, icon: 'plus', action: { call: 'apps.launch', params: { id: g.app.id } } });
      if (g.windows.length > 1) {
        items.push({ label: '', separator: true });
        for (const w of g.windows) {
          items.push({ label: `${w.minimized ? '· ' : ''}${w.title || g.label}`, icon: ORIGIN_ICON[originOf(w)], action: { call: 'windows.focus', params: { id: w.id } } });
        }
      }
      if (g.windows.length === 1) {
        const w = g.windows[0];
        items.push({ label: '', separator: true });
        items.push({ label: w.minimized ? 'Restore' : 'Minimise', icon: w.minimized ? 'restore' : 'minimize', action: { call: w.minimized ? 'windows.unminimize' : 'windows.minimize', params: { id: w.id } } });
        items.push({ label: w.maximized ? 'Unmaximise' : 'Maximise', icon: 'maximize', action: { call: 'windows.toggleMaximize', params: { id: w.id } } });
        items.push({ label: w.fullscreen ? 'Leave full screen' : 'Full screen', icon: 'display', action: { call: 'windows.toggleFullscreen', params: { id: w.id } } });
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

    const createItem = (g: Group) => {
      const img = h('img', { class: 'task-ic', src: iconFor(g), alt: '', draggable: false });
      const badge = h('span', { class: 'task-badge' });
      const icon0 = h('span', { class: 'task-ic-wrap' }, img, badge);
      const label = h('span', { class: 'task-label' });
      const count = h('span', { class: 'task-count' });
      const dots = h('span', { class: 'task-dots' });
      const item = h('div', { class: 'task', tabindex: -1 }, icon0, label, count, dots);
      const live = () => current.get(g.key) ?? g;
      item.addEventListener('click', (e) => onClick(live(), e));
      item.addEventListener('auxclick', (e) => {
        if (e.button === 1) onClick(live(), e);
      });
      item.addEventListener('contextmenu', (e) => onContext(live(), e));
      item.addEventListener('wheel', (e) => onWheel(live(), e), { passive: false });
      item.addEventListener('pointerenter', () => armCard(live(), item));
      item.addEventListener('pointerleave', disarm);
      return item;
    };

    const updateItem = (item: HTMLElement, g: Group) => {
      const wrap = item.firstElementChild!;
      const img = wrap.firstElementChild as HTMLImageElement;
      const src = iconFor(g);
      if (img.getAttribute('src') !== src) img.src = src;
      const badge = wrap.lastElementChild as HTMLElement;
      const mode = String(cfg.osBadge ?? 'foreign');
      const wantBadge = mode === 'all' || (mode !== 'off' && g.origin !== 'wayland');
      if (wantBadge && badge.dataset.g !== g.origin) {
        badge.dataset.g = g.origin;
        badge.innerHTML = iconSvg(ORIGIN_ICON[g.origin], 9);
        badge.title = ORIGIN_NAME[g.origin];
      }
      badge.classList.toggle('on', wantBadge);
      const label = item.children[1] as HTMLElement;
      const focusedWin = g.windows.find((w) => w.focused);
      const text = cfg.labels && g.windows.length ? (focusedWin ?? g.windows[0])?.title || g.label : '';
      if (label.textContent !== text) label.textContent = text;
      label.style.maxWidth = `${Number(cfg.maxLabel) || 160}px`;
      const count = item.children[2] as HTMLElement;
      const ctext = g.windows.length > 1 ? String(g.windows.length) : '';
      if (count.textContent !== ctext) count.textContent = ctext;
      const minimized = g.windows.length > 0 && g.windows.every((w) => w.minimized);
      const lines = g.windows.length ? g.windows.map((w) => `${w.minimized ? '· ' : ''}${w.title || g.label}`) : [g.label];
      if (!g.windows.length && g.elsewhere) lines.push(`${g.elsewhere} open`);
      lines.push(ORIGIN_NAME[g.origin]);
      item.title = lines.join('\n');
      item.classList.toggle('wine', g.origin === 'wine');
      item.classList.toggle('running', g.windows.length > 0 || g.elsewhere > 0);
      item.classList.toggle('has-windows', g.windows.length > 0);
      item.classList.toggle('active', g.windows.some((w) => w.focused && !w.minimized));
      item.classList.toggle('minimized', minimized);
      item.classList.toggle('pinned', g.pinned);
      const dots = item.children[3] as HTMLElement;
      const marks = g.windows.length ? g.windows.slice(0, 4) : g.elsewhere ? [undefined] : [];
      if (dots.childElementCount !== marks.length) {
        dots.textContent = '';
        for (let i = 0; i < marks.length; i++) dots.appendChild(h('i'));
      }
      marks.forEach((w, i) => {
        const dot = dots.children[i] as HTMLElement;
        dot.className = w ? (w.focused && !w.minimized ? 'on' : w.minimized ? 'min' : '') : 'elsewhere';
      });
    };

    const render = () => {
      const state = ctx.store.state;
      const pins = Array.isArray(cfg.pins) ? (cfg.pins as string[]) : [];
      const merge = !!cfg.mergePinned;
      const onlyHere = cfg.onlyThisOutput || ['columns', 'dwindle'].includes(ctx.store.layoutMode?.mode ?? '');
      const windows = cfg.showRunning === false ? [] : state.windows.filter((w) => !onlyHere || !w.output || w.output === ctx.output);
      const { pinned, running } = buildGroups(state, pins, windows, merge);
      el.classList.toggle('labels', !!cfg.labels && cfg.showRunning !== false);
      el.classList.toggle('no-indicator', cfg.indicator === false);
      el.classList.toggle('split', !merge && cfg.showRunning !== false);
      const size = Math.max(0, Number(cfg.iconSize) || 0);
      if (size) el.style.setProperty('--task-ic', `${size}px`);
      else el.style.removeProperty('--task-ic');
      current = new Map([...pinned, ...running].map((g) => [g.key, g]));
      reconcile(pinsEl, pinned, (g) => g.key, createItem, updateItem);
      reconcile(runEl, running, (g) => g.key, createItem, updateItem);
      // A seam only means something with something on both sides of it.
      sepEl.hidden = merge || cfg.showRunning === false || cfg.separator === false || !pinned.length || !running.length;
      // With nothing pinned and nothing open the widget would be an invisible
      // gap on the bar; say what it is instead.
      emptyEl.hidden = !(pinned.length === 0 && running.length === 0 && cfg.showRunning !== false);
      refreshCard();
    };

    render();
    ctx.store.bind(el, 'windows', render);
    ctx.store.bind(el, 'apps', render);
    ctx.store.bind(el, 'layoutMode', render);
    if (!ctx.store.layoutMode) void ctx.store.fetchLayoutMode();
    return {
      el,
      update(c) {
        cfg = c;
        render();
      },
      destroy() {
        disarm();
        ctx.flyout?.hide();
      },
    };
  },
});
