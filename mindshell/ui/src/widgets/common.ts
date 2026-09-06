// Shared bits for panel widgets.

import { h } from '../dom';
import type { WidgetCtx } from './registry';

/** The standard clickable panel item: an icon, optional text, hover tint. */
export function panelItem(ctx: WidgetCtx, cls: string, title?: string): HTMLElement {
  return h('div', { class: `w w-${ctx.type} ${cls}`.trim(), title, tabindex: -1 });
}

export function outputPoint(ctx: WidgetCtx, e: MouseEvent): { x: number; y: number } {
  const o = ctx.origin();
  return { x: o.x + e.clientX, y: o.y + e.clientY };
}

export function pct(v: number): string {
  return `${Math.round(v)}%`;
}
