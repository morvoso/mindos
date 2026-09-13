import * as bridge from '../bridge';
import { h, reconcile } from '../dom';
import { registerWidget } from './registry';
import { outputPoint } from './common';
import type { TrayItem } from '../types';

registerWidget({
  type: 'tray',
  name: 'System tray',
  description: 'Status icons from running applications (StatusNotifier).',
  icon: 'box',
  containers: ['panel'],
  defaults: { hidePassive: true, iconSize: 0 },
  settings: {
    hidePassive: { label: 'Hide passive items', type: 'boolean', help: 'Icons whose status is passive (no attention required)' },
    iconSize: { label: 'Icon size', type: 'number', min: 0, max: 28, step: 2, unit: 'px', help: '0 follows the panel: a taller bar gets bigger icons' },
  },
  create(ctx) {
    const el = h('div', { class: 'w w-tray' });
    let cfg = ctx.config;
    const render = () => {
      const items = ctx.store.state.tray.filter((t) => !(cfg.hidePassive && t.status === 'passive'));
      el.classList.toggle('empty', items.length === 0);
      // 0 (the default) leaves `--tray-ic` to app.css, which derives it from
      // the panel's thickness; any other value pins the icon to that size.
      const fixed = Math.max(0, Number(cfg.iconSize) || 0);
      if (fixed) el.style.setProperty('--tray-ic', `${fixed}px`);
      else el.style.removeProperty('--tray-ic');
      reconcile(
        el,
        items,
        (t) => t.id,
        (t) => {
          const img = h('img', { class: 'tray-ic', alt: '', draggable: false });
          const item = h('div', { class: 'tray-item', tabindex: -1 }, img);
          const cur = () => ctx.store.state.tray.find((x) => x.id === t.id) ?? t;
          item.addEventListener('click', (e) => {
            const p = outputPoint(ctx, e);
            bridge.send('tray.activate', { id: cur().id, x: p.x, y: p.y });
          });
          item.addEventListener('auxclick', (e) => {
            if (e.button !== 1) return;
            const p = outputPoint(ctx, e);
            bridge.send('tray.secondaryActivate', { id: cur().id, x: p.x, y: p.y });
          });
          item.addEventListener('contextmenu', (e) => {
            e.preventDefault();
            const it = cur();
            if (it.hasMenu) ctx.openPopup('tray-menu', { id: it.id, title: it.title, anchor: ctx.anchorOf(item) });
            else {
              const p = outputPoint(ctx, e);
              bridge.send('tray.secondaryActivate', { id: it.id, x: p.x, y: p.y });
            }
          });
          item.addEventListener(
            'wheel',
            (e) => {
              e.preventDefault();
              const horizontal = Math.abs(e.deltaX) > Math.abs(e.deltaY);
              const d = horizontal ? e.deltaX : e.deltaY;
              bridge.send('tray.scroll', { id: cur().id, delta: Math.sign(d) * 120, orientation: horizontal ? 'horizontal' : 'vertical' });
            },
            { passive: false },
          );
          return item;
        },
        (item, t: TrayItem) => {
          const img = item.firstElementChild as HTMLImageElement;
          if (img.getAttribute('src') !== t.icon) img.src = t.icon;
          item.title = t.tooltip || t.title;
          item.classList.toggle('attention', t.status === 'attention');
        },
      );
    };
    render();
    ctx.store.bind(el, 'tray', render);
    return {
      el,
      update(c) {
        cfg = c;
        render();
      },
    };
  },
});
