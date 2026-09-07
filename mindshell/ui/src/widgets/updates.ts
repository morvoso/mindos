// The updates indicator: what the Mind's update watcher found, in the bar
// next to the tray. Nothing waiting, nothing shown; click for Settings ›
// Updates, where the Mind explains the risk and applies them.

import { openApp } from '../apps/shared';
import { h } from '../dom';
import { icon } from '../icons';
import { registerWidget } from './registry';
import { panelItem } from './common';

registerWidget({
  type: 'updates',
  name: 'Updates',
  description: 'How many updates are waiting and what the Mind thinks of them. Hidden while the system is up to date.',
  icon: 'package',
  containers: ['panel'],
  defaults: { count: true, alwaysShow: false },
  settings: {
    count: { label: 'Show the count', type: 'boolean', help: 'The number of packages waiting to be installed' },
    alwaysShow: { label: 'Show when up to date', type: 'boolean', help: 'When off, the icon appears only when updates are available' },
  },
  create(ctx) {
    const el = panelItem(ctx, 'w-updates', 'Updates');
    const ic = h('span', { class: 'w-ic' }, icon('package', 18));
    const badge = h('span', { class: 'w-badge' });
    el.append(ic, badge);
    let cfg = ctx.config;
    const render = () => {
      const u = ctx.store.state.mind?.updates;
      const n = u?.packages.length ?? 0;
      // The Mind flags what it would not apply on its own.
      const careful = !!u && (u.risk === 'high' || u.manual_intervention);
      el.classList.toggle('hidden', n === 0 && !cfg.alwaysShow);
      el.classList.toggle('has', n > 0);
      el.classList.toggle('warn', careful);
      badge.textContent = n > 99 ? '99+' : String(n);
      badge.hidden = !cfg.count || n === 0;
      el.title = n === 0
        ? 'Up to date'
        : `${n} update${n === 1 ? '' : 's'} waiting${u?.risk ? ` · ${u.risk} risk` : ''}${u?.manual_intervention ? ' · manual steps required' : ''}`;
    };
    render();
    ctx.store.bind(el, 'mindUpdates', render);
    ctx.store.bind(el, 'mind', render);
    el.addEventListener('click', () => openApp('settings', 'updates'));
    return {
      el,
      update(c) {
        cfg = c;
        render();
      },
    };
  },
});
