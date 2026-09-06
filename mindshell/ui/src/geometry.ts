// Window geometry shared by the panel renderer, the desktop and the preview.

import { clamp } from './dom';
import type { OutputInfo, PanelDef } from './types';

/** Extra thickness a panel window gains in edit mode (the settings strip). */
export const EDIT_EXTRA = 140;

export interface Rect {
  x: number;
  y: number;
  w: number;
  h: number;
}

export function isVertical(p: PanelDef): boolean {
  return p.edge === 'left' || p.edge === 'right';
}

/** Panels that apply to an output: its own plus the ones for every output ('*'). */
export function panelsOn(all: PanelDef[], output: string): PanelDef[] {
  return all.filter((p) => p.output === '*' || p.output === output);
}

/**
 * Where the panel's window sits on its output (output-local coordinates).
 * Horizontal panels span the full edge; vertical panels are inset by the
 * exclusive zones of the horizontal panels on the same output, which is the
 * order the host creates the layer surfaces in. A panel with length 0 fits
 * its widgets: `fitLen` is the measured length (a third of the edge until it
 * is known); in edit mode such a panel temporarily spans the whole edge so
 * the settings strip has room.
 */
export function panelWindowRect(p: PanelDef, out: OutputInfo, editing: boolean, all: PanelDef[] = [], fitLen?: number): Rect {
  const vertical = isVertical(p);
  const thick = p.size + (editing ? EDIT_EXTRA : 0);
  let inset0 = 0;
  let inset1 = 0;
  if (vertical) {
    for (const o of panelsOn(all, out.name)) {
      if (o.id === p.id || isVertical(o)) continue;
      const zone = o.margin + o.size;
      if (o.edge === 'top') inset0 = Math.max(inset0, zone);
      else inset1 = Math.max(inset1, zone);
    }
  }
  const span = (vertical ? out.height : out.width) - inset0 - inset1;
  const fit = p.length <= 0;
  let len: number;
  if (fit && !editing) len = Math.round(clamp(fitLen ?? span / 3, 1, span));
  else if (fit) len = span;
  else len = Math.round((span * clamp(p.length, 10, 100)) / 100);
  const offset = inset0 + (fit || p.align === 'center' ? Math.round((span - len) / 2) : p.align === 'start' ? 0 : span - len);
  if (vertical) {
    const x = p.edge === 'left' ? p.margin : out.width - p.margin - thick;
    return { x, y: offset, w: thick, h: len };
  }
  const y = p.edge === 'top' ? p.margin : out.height - p.margin - thick;
  return { x: offset, y, w: len, h: thick };
}

/** Scale applied to `root` by an ancestor transform (the preview stage); 1 in a real window. */
export function rootScale(root: HTMLElement): number {
  const w = root.offsetWidth;
  if (!w) return 1;
  return root.getBoundingClientRect().width / w || 1;
}

/** Rectangle of `el` relative to `root`, in unscaled CSS pixels. */
export function rectIn(root: HTMLElement, el: Element): Rect {
  const s = rootScale(root);
  const r = el.getBoundingClientRect();
  const b = root.getBoundingClientRect();
  return { x: (r.left - b.left) / s, y: (r.top - b.top) / s, w: r.width / s, h: r.height / s };
}
