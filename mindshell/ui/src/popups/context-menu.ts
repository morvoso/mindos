import { runAction } from '../actions';
import { h } from '../dom';
import { icon } from '../icons';
import type { MenuAction } from '../types';
import type { PopupContent, PopupCtx } from './shared';

export function menuList(items: MenuAction[], onPick: (a: MenuAction) => void): HTMLElement {
  const list = h('div', { class: 'menu' });
  for (const it of items) {
    if (it.separator) {
      list.appendChild(h('div', { class: 'menu-sep' }));
      continue;
    }
    const btn = h('button', { class: `menu-item${it.danger ? ' danger' : ''}`, disabled: !!it.disabled }, h('span', { class: 'menu-ic' }, it.icon ? icon(it.icon, 14) : null), h('span', { class: 'menu-label' }, it.label));
    btn.addEventListener('click', () => onPick(it));
    list.appendChild(btn);
  }
  return list;
}

export function contextMenuPopup(ctx: PopupCtx): PopupContent {
  const items = (ctx.arg.items as MenuAction[] | undefined) ?? [];
  const title = ctx.arg.title as string | undefined;
  const el = h('div', { class: 'pop-body menu-pop' });
  if (title) el.appendChild(h('div', { class: 'menu-title' }, title));
  el.appendChild(
    menuList(items, (it) => {
      ctx.close();
      void runAction(it.action);
    }),
  );
  return { el };
}
