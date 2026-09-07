// Layout helpers: the default layout, lookups and small mutations.

import { deepClone, newId } from './dom';
import type { DesktopWidgetEntry, Layout, PanelDef, WidgetEntry } from './types';

export function defaultLayout(): Layout {
  return {
    version: 2,
    panels: [
      {
        // One bar along the bottom, flush with the edge: the apps centred,
        // the tray, Mind and the clock at the right.
        id: 'bar', output: '*', edge: 'bottom', size: 48, length: 100, align: 'center', margin: 0, layer: 'top', opacity: 0.85, float: false,
        widgets: [
          { id: 'sp-l', type: 'spacer', config: { expand: true } },
          { id: 'tasks', type: 'taskbar', config: { pins: ['firefox.desktop', 'org.gnome.Nautilus.desktop', 'foot.desktop', 'steam.desktop', 'mindos-settings.desktop'] } },
          { id: 'sp-r', type: 'spacer', config: { expand: true } },
          { id: 'tray', type: 'tray', config: {} },
          { id: 'audio', type: 'audio', config: {} },
          { id: 'net', type: 'network', config: {} },
          { id: 'bat', type: 'battery', config: {} },
          { id: 'mode', type: 'layout-mode', config: {} },
          { id: 'perf', type: 'perf', config: {} },
          { id: 'mind', type: 'mind', config: {} },
          { id: 'notify', type: 'notifications', config: {} },
          { id: 'clock', type: 'clock', config: { seconds: false, date: true, hour24: false } },
        ],
      },
    ],
    desktop: {
      wallpaper: { mode: 'builtin' },
      icons: true,
      widgets: [],
    },
  };
}

/** Panel length: 0 (or less) means "fit the widgets", otherwise a percentage of the edge. */
export function panelLength(raw: unknown): number {
  if (raw === undefined || raw === null || raw === '') return 100;
  const n = Number(raw);
  if (Number.isNaN(n)) return 100;
  return n <= 0 ? 0 : Math.min(100, Math.max(10, n));
}

export function isFitPanel(p: { length: number }): boolean {
  return p.length <= 0;
}

/** Fill in anything a hand-edited or older layout file may lack. */
export function normalizeLayout(raw: Partial<Layout> | null | undefined): Layout {
  const base = defaultLayout();
  if (!raw || typeof raw !== 'object') return base;
  const panels = Array.isArray(raw.panels) ? raw.panels : base.panels;
  const desktop = raw.desktop && typeof raw.desktop === 'object' ? raw.desktop : base.desktop;
  return {
    version: 2,
    panels: panels.map((p, i) => ({
      id: p.id ?? `panel-${i}`,
      output: p.output ?? '*',
      edge: p.edge ?? 'bottom',
      size: Number(p.size) || 40,
      length: panelLength(p.length),
      align: p.align ?? 'center',
      margin: Number(p.margin) || 0,
      layer: p.layer ?? 'top',
      opacity: typeof p.opacity === 'number' ? p.opacity : 0.92,
      ...(typeof p.float === 'boolean' ? { float: p.float } : {}),
      widgets: (p.widgets ?? []).map((w, j) => ({ id: w.id ?? `w-${i}-${j}`, type: w.type ?? 'unknown', config: w.config ?? {} })),
    })),
    desktop: {
      wallpaper: desktop.wallpaper ?? { mode: 'builtin' },
      icons: desktop.icons !== false,
      widgets: (desktop.widgets ?? []).map((w, j) => ({
        id: w.id ?? `d-${j}`, type: w.type ?? 'unknown', output: w.output ?? '*',
        x: Number(w.x) || 0, y: Number(w.y) || 0, w: Number(w.w) || 240, h: Number(w.h) || 120, config: w.config ?? {},
      })),
    },
  };
}

export function panelById(layout: Layout, id: string): PanelDef | undefined {
  return layout.panels.find((p) => p.id === id);
}

export function panelsForOutput(layout: Layout, output: string): PanelDef[] {
  return layout.panels.filter((p) => p.output === '*' || p.output === output);
}

export function desktopWidgetsForOutput(layout: Layout, output: string): DesktopWidgetEntry[] {
  return layout.desktop.widgets.filter((w) => w.output === '*' || w.output === output);
}

export function newPanel(edge: PanelDef['edge'], existing: PanelDef[]): PanelDef {
  const vertical = edge === 'left' || edge === 'right';
  let id: string = edge;
  let n = 2;
  while (existing.some((p) => p.id === id)) id = `${edge}-${n++}`;
  return {
    id, output: '*', edge, size: vertical ? 48 : 36, length: 100, align: 'center', margin: 0, layer: 'top', opacity: 0.92,
    widgets: [{ id: newId('sp'), type: 'spacer', config: { expand: true } }],
  };
}

export function newWidget(type: string, config: Record<string, unknown> = {}): WidgetEntry {
  return { id: newId(type), type, config: deepClone(config) };
}
