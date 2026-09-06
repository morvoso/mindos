import * as bridge from '../bridge';
import { h } from '../dom';
import { icon } from '../icons';
import type { MenuItem } from '../types';
import type { PopupContent, PopupCtx } from './shared';

export function trayMenuPopup(ctx: PopupCtx): PopupContent {
  const id = String(ctx.arg.id ?? '');
  const el = h('div', { class: 'pop-body menu-pop' });
  if (ctx.arg.title) el.appendChild(h('div', { class: 'menu-title' }, String(ctx.arg.title)));
  const list = h('div', { class: 'menu' }, h('div', { class: 'menu-empty' }, 'Loading…'));
  el.appendChild(list);

  const renderItems = (items: MenuItem[], depth: number): HTMLElement[] => {
    const nodes: HTMLElement[] = [];
    for (const it of items) {
      if (it.type === 'separator') {
        nodes.push(h('div', { class: 'menu-sep' }));
        continue;
      }
      const mark = it.toggle === 'checkmark' ? (it.checked ? icon('check', 14) : null) : it.toggle === 'radio' ? h('i', { class: `radio${it.checked ? ' on' : ''}` }) : it.icon ? h('img', { src: it.icon, alt: '' }) : null;
      const btn = h('button', { class: 'menu-item', disabled: !it.enabled, style: { paddingLeft: `${12 + depth * 14}px` } }, h('span', { class: 'menu-ic' }, mark), h('span', { class: 'menu-label' }, it.label.replace(/_(.)/g, '$1')));
      if (it.type === 'submenu' && it.children?.length) {
        btn.appendChild(icon('chevron-right', 12, 'caret'));
        const sub = h('div', { class: 'menu-sub', hidden: true }, ...renderItems(it.children, depth + 1));
        btn.addEventListener('click', () => {
          sub.hidden = !sub.hidden;
          btn.classList.toggle('open', !sub.hidden);
          ctx.relayout();
        });
        nodes.push(btn, sub);
      } else {
        btn.addEventListener('click', () => {
          bridge.send('tray.menuClick', { id, item: it.id });
          ctx.close();
        });
        nodes.push(btn);
      }
    }
    return nodes;
  };

  bridge
    .call<{ items: MenuItem[] } | MenuItem[]>('tray.menu', { id })
    .then((res) => {
      const items = Array.isArray(res) ? res : res?.items ?? [];
      list.replaceChildren(...(items.length ? renderItems(items, 0) : [h('div', { class: 'menu-empty' }, 'No menu')]));
      ctx.relayout();
    })
    .catch(() => {
      list.replaceChildren(h('div', { class: 'menu-empty' }, 'Menu unavailable'));
      ctx.relayout();
    });
  return { el };
}
