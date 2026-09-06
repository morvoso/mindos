// Popup router: picks the popup by name, places it next to its anchor and
// closes it on Escape or a click outside.

import { clamp, h } from '../dom';
import { rootScale } from '../geometry';
import { store } from '../state';
import type { Anchor } from '../types';
import { audioPopup } from './audio';
import { calendarPopup } from './calendar';
import { contextMenuPopup } from './context-menu';
import { layoutModePopup } from './layout-mode';
import { powerPopup } from './power';
import type { PopupContent, PopupCtx, PopupFactory } from './shared';
import { trayMenuPopup } from './tray-menu';
import { widgetCatalogPopup } from './widget-catalog';
import { widgetSettingsPopup } from './widget-settings';

export const POPUPS: Record<string, PopupFactory> = {
  calendar: calendarPopup,
  'layout-mode': layoutModePopup,
  audio: audioPopup,
  power: powerPopup,
  'context-menu': contextMenuPopup,
  'tray-menu': trayMenuPopup,
  'widget-catalog': widgetCatalogPopup,
  'widget-settings': widgetSettingsPopup,
};

const GAP = 8;
const MARGIN = 8;

export function placePopup(anchor: Anchor | undefined, w: number, h: number, out: { width: number; height: number }): { x: number; y: number; origin: string } {
  const maxX = Math.max(MARGIN, out.width - w - MARGIN);
  const maxY = Math.max(MARGIN, out.height - h - MARGIN);
  if (!anchor) return { x: Math.round((out.width - w) / 2), y: Math.round((out.height - h) / 2), origin: 'center' };
  let x = anchor.x;
  let y = anchor.y;
  let origin = 'top left';
  switch (anchor.edge) {
    case 'bottom':
      y = anchor.y - h - GAP;
      origin = 'bottom left';
      break;
    case 'top':
      y = anchor.y + anchor.h + GAP;
      break;
    case 'left':
      x = anchor.x + anchor.w + GAP;
      break;
    case 'right':
      x = anchor.x - w - GAP;
      origin = 'top right';
      break;
    default:
      // A pointer position: open below-right, flipping when there is no room.
      x = anchor.x + 2;
      y = anchor.y + 2;
      if (y + h > out.height - MARGIN) {
        y = anchor.y - h - 2;
        origin = 'bottom left';
      }
      if (x + w > out.width - MARGIN) x = anchor.x - w - 2;
  }
  if (anchor.edge === 'bottom' || anchor.edge === 'top') {
    // Centre on the anchor when it is narrower than the popup.
    if (anchor.w < w) x = anchor.x + anchor.w / 2 - w / 2;
    if (x + w > out.width - MARGIN) origin = origin.replace('left', 'right');
  }
  return { x: Math.round(clamp(x, MARGIN, maxX)), y: Math.round(clamp(y, MARGIN, maxY)), origin };
}

export function renderPopupWindow(root: HTMLElement, name: string, arg: unknown, output: string, close: () => void): () => void {
  root.classList.add('popup-window');
  const a = (arg && typeof arg === 'object' ? arg : {}) as Record<string, unknown>;
  const anchor = a.anchor as Anchor | undefined;
  const outInfo = store.output(output);
  const out = { width: outInfo?.width ?? root.offsetWidth, height: outInfo?.height ?? root.offsetHeight };
  const pop = h('div', { class: `pop pop-${name}`, role: 'dialog' });
  const backdrop = h('div', { class: 'pop-backdrop' });
  let closed = false;
  const doClose = () => {
    if (closed) return;
    closed = true;
    close();
  };
  let content: PopupContent;
  let placeFn: (() => void) | undefined;
  const ctx: PopupCtx = { name, arg: a, output, anchor, store, close: doClose, relayout: () => placeFn?.() };
  const factory = POPUPS[name];
  try {
    content = factory ? factory(ctx) : { el: h('div', { class: 'pop-body' }, `Unknown popup “${name}”`), w: 260 };
  } catch (e) {
    console.error(`popup ${name} failed`, e);
    content = { el: h('div', { class: 'pop-body' }, `${name} failed to load`), w: 260 };
  }
  pop.appendChild(content.el);
  if (content.w) pop.style.width = `${content.w}px`;
  if (content.h) pop.style.height = `${content.h}px`;
  pop.style.maxWidth = `${out.width - MARGIN * 2}px`;
  pop.style.maxHeight = `${out.height - MARGIN * 2}px`;
  root.append(backdrop, pop);

  const place = () => {
    const s = rootScale(root);
    const r = pop.getBoundingClientRect();
    const w = content.w ?? r.width / s;
    const hh = content.h ?? r.height / s;
    const p = placePopup(anchor, w, hh, out);
    pop.style.left = `${p.x}px`;
    pop.style.top = `${p.y}px`;
    pop.style.transformOrigin = p.origin;
  };
  placeFn = place;
  place();
  requestAnimationFrame(() => pop.classList.add('in'));

  backdrop.addEventListener('pointerdown', doClose);
  const onKey = (e: KeyboardEvent) => {
    if (e.key === 'Escape') {
      e.preventDefault();
      doClose();
    }
  };
  root.addEventListener('keydown', onKey);
  window.addEventListener('keydown', onKey);
  content.focus?.();
  return () => {
    window.removeEventListener('keydown', onKey);
    root.removeEventListener('keydown', onKey);
    backdrop.remove();
    pop.remove();
  };
}
