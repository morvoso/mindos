// Layout helpers: the default layout, lookups and small mutations.

import { deepClone, newId } from './dom';
import type { AppearancePalette, DesktopWidgetEntry, Layout, PanelDef, WidgetEntry, WorkspaceShortcut } from './types';
// The one copy of the shipped layout. The host compiles the same file into the
// binary and installs it as /usr/share/mindos/shell/layout.json, so there is
// nothing here to keep in sync by hand.
import builtin from '../../data/layout.json';

export function defaultLayout(): Layout {
  return deepClone(builtin as unknown as Layout);
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
    version: 4,
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
      workspace: {
        mode: desktop.workspace?.mode === 'productivity' ? 'productivity' : 'gaming',
        notes: String(desktop.workspace?.notes ?? ''),
        activate: desktop.workspace?.activate === 'double' ? 'double' : 'single',
        shortcuts: Array.isArray(desktop.workspace?.shortcuts) ? desktop.workspace.shortcuts.map((s: WorkspaceShortcut) => ({
          id: String(s.id), appId: String(s.appId), label: String(s.label),
          ...(s.icon ? { icon: String(s.icon) } : {}), ...(s.pinned ? { pinned: true } : {}),
        })) : [],
      },
      ...(desktop.appearance ? {
        appearance: {
          theme: desktop.appearance.theme === 'light' ? 'light' as const : 'dark' as const,
          ...(desktop.appearance.live !== undefined ? { live: Boolean(desktop.appearance.live) } : {}),
          ...(desktop.appearance.dark ? { dark: desktop.appearance.dark as AppearancePalette } : {}),
          ...(desktop.appearance.light ? { light: desktop.appearance.light as AppearancePalette } : {}),
          ...(desktop.appearance.preset ? { preset: String(desktop.appearance.preset) } : {}),
          ...(Array.isArray(desktop.appearance.saved) ? { saved: desktop.appearance.saved } : {}),
        },
      } : {}),
      ...(desktop.library ? { library: desktop.library } : {}),
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
