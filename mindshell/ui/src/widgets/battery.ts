import * as bridge from '../bridge';
import { every, h } from '../dom';
import { batteryIcon } from '../icons';
import { registerWidget } from './registry';
import { panelItem, pct } from './common';
import type { BatteryState } from '../types';

registerWidget({
  type: 'battery',
  name: 'Battery',
  description: 'Charge level; hidden on machines without a battery.',
  icon: 'battery',
  containers: ['panel'],
  defaults: { percent: true, warnAt: 15, alwaysShow: false },
  settings: {
    percent: { label: 'Show the percentage', type: 'boolean' },
    warnAt: { label: 'Warn below', type: 'number', min: 5, max: 50, step: 5, unit: '%' },
    alwaysShow: { label: 'Show without a battery', type: 'boolean', help: 'Hidden by default on systems without a battery' },
  },
  create(ctx) {
    const el = panelItem(ctx, 'w-battery', 'Battery');
    const ic = h('span', { class: 'w-ic' });
    const label = h('span', { class: 'w-val mono' });
    el.append(ic, label);
    let cfg = ctx.config;
    let state: BatteryState | undefined;
    const render = () => {
      const present = !!state?.present;
      el.classList.toggle('hidden', !present && !cfg.alwaysShow);
      if (!present) {
        ic.replaceChildren(batteryIcon(0, false, 18));
        label.textContent = '--';
        label.hidden = !cfg.percent || !!ctx.panel?.vertical;
        el.classList.remove('low', 'charging');
        el.title = 'No battery';
        return;
      }
      const b = state!;
      const p = b.percent ?? 0;
      const charging = !!b.charging;
      ic.replaceChildren(batteryIcon(p, charging, 18));
      label.textContent = pct(p);
      label.hidden = !cfg.percent || !!ctx.panel?.vertical;
      el.classList.toggle('low', p <= (Number(cfg.warnAt) || 15) && !charging);
      el.classList.toggle('charging', charging);
      const left = b.timeToEmpty ? ` · ${Math.floor(b.timeToEmpty / 3600)}h ${Math.round((b.timeToEmpty % 3600) / 60)}m left` : '';
      el.title = `${pct(p)}${charging ? ' · charging' : ''}${left}`;
    };
    const poll = () =>
      bridge.call<BatteryState>('battery.status').then((s) => {
        state = s;
        render();
      }).catch(() => undefined);
    render();
    poll();
    every(el, 30000, poll);
    return {
      el,
      update(c) {
        cfg = c;
        render();
      },
    };
  },
});
