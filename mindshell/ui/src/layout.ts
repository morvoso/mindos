// Layout helpers: the default layout, lookups and small mutations.

import { deepClone, newId } from './dom';
import type { AppearancePalette, DesktopWidgetEntry, Layout, PanelDef, PerfMode, Space, WidgetEntry, WorkspaceShortcut } from './types';
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
  const panels = (Array.isArray(raw.panels) ? raw.panels : base.panels).slice();
  const desktop = raw.desktop && typeof raw.desktop === 'object' ? raw.desktop : base.desktop;
  // 5 added the view indicator. It is the only thing that says which of the two
  // desktop views the screen is in, and a layout saved before it existed has no
  // way to know to ask for it, so it is put in the first panel for them.
  if ((Number(raw.version) || 0) < 5 && panels.length && !panels.some((p) => (p.widgets ?? []).some((w) => w.type === 'desktop-view'))) {
    panels[0] = { ...panels[0], widgets: [{ id: 'view', type: 'desktop-view', config: {} }, ...(panels[0].widgets ?? [])] };
  }
  return {
    version: 5,
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
      workspace: normalizeWorkspace(desktop.workspace),
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

type Workspace = NonNullable<Layout['desktop']['workspace']>;
const PERF: PerfMode[] = ['balanced', 'performance', 'quiet'];
const LAYOUT_MODES = ['floating', 'dwindle', 'columns'];

function normalizeShortcuts(raw: unknown): WorkspaceShortcut[] | undefined {
  if (!Array.isArray(raw)) return undefined;
  return raw.filter((s) => s && typeof s === 'object').map((s: WorkspaceShortcut) => ({
    id: String(s.id), appId: String(s.appId), label: String(s.label),
    ...(s.icon ? { icon: String(s.icon) } : {}), ...(s.pinned ? { pinned: true } : {}),
  }));
}

/** The spaces a layout from before them had: its two modes, what each asked
 *  of the machine, and the one set of notes and shortcuts going to Work. */
function legacySpaces(old: Record<string, unknown>): Space[] {
  return [
    { id: 'gaming', name: 'Gaming', icon: 'gamepad', perf: 'performance', recent: true, notes: '' },
    { id: 'work', name: 'Work', icon: 'grid', perf: 'balanced', notes: String(old.notes ?? ''), ...(Array.isArray(old.shortcuts) ? { shortcuts: normalizeShortcuts(old.shortcuts) } : {}) },
  ];
}

export function normalizeWorkspace(raw: unknown): Workspace {
  const old = (raw && typeof raw === 'object' ? raw : {}) as Record<string, unknown>;
  const seen = new Set<string>();
  let spaces: Space[] = Array.isArray(old.spaces) ? (old.spaces as Partial<Space>[]).filter((s) => s && typeof s === 'object').map((s, i) => {
    let id = String(s.id || `space-${i}`);
    while (seen.has(id)) id = `${id}-${i}`;
    seen.add(id);
    const shortcuts = normalizeShortcuts(s.shortcuts);
    return {
      id, name: String(s.name ?? '').slice(0, 40) || 'Space',
      ...(s.icon ? { icon: String(s.icon) } : {}),
      ...(PERF.includes(s.perf as PerfMode) ? { perf: s.perf } : {}),
      ...(typeof s.palette === 'string' && /^(preset|saved):./.test(s.palette) ? { palette: s.palette } : {}),
      ...(LAYOUT_MODES.includes(String(s.layoutMode)) ? { layoutMode: String(s.layoutMode) } : {}),
      ...(s.recent ? { recent: true } : {}),
      notes: String(s.notes ?? ''),
      ...(shortcuts ? { shortcuts } : {}),
    };
  }) : [];
  let space = String(old.space ?? '');
  if (!spaces.length) {
    spaces = legacySpaces(old);
    space = old.mode === 'productivity' ? 'work' : 'gaming';
  }
  if (!spaces.some((s) => s.id === space)) space = spaces[0].id;
  return {
    space, spaces,
    activate: old.activate === 'double' ? 'double' : 'single',
    sticky: Array.isArray(old.sticky) ? [...new Set((old.sticky as unknown[]).map(String).filter(Boolean))] : [],
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
