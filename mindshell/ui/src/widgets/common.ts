// Shared bits for panel widgets.

import { h } from '../dom';
import type { WidgetCtx } from './registry';

/**
 * The standard panel item. `tap` marks it as a clickable surface: only those
 * light up under the pointer, so a spacer, a meter or a read-out never
 * pretends there is something to press.
 */
export function panelItem(ctx: WidgetCtx, cls: string, title?: string, tap = true): HTMLElement {
  return h('div', { class: `w${tap ? ' tap' : ''} w-${ctx.type} ${cls}`.trim(), title, tabindex: -1 });
}

export function outputPoint(ctx: WidgetCtx, e: MouseEvent): { x: number; y: number } {
  const o = ctx.origin();
  return { x: o.x + e.clientX, y: o.y + e.clientY };
}

export function pct(v: number): string {
  return `${Math.round(v)}%`;
}
