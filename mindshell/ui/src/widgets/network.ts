import * as bridge from '../bridge';
import { every, h } from '../dom';
import { icon } from '../icons';
import { registerWidget } from './registry';
import { panelItem } from './common';
import type { NetworkState } from '../types';

registerWidget({
  type: 'network',
  name: 'Network',
  description: 'Connection status. Shows the network name when connected.',
  icon: 'wifi',
  containers: ['panel'],
  defaults: { name: false },
  settings: { name: { label: 'Show the network name', type: 'boolean' } },
  create(ctx) {
    const el = panelItem(ctx, 'w-network', 'Network');
    const ic = h('span', { class: 'w-ic' });
    const label = h('span', { class: 'w-label' });
    el.append(ic, label);
    let cfg = ctx.config;
    let state: NetworkState | undefined;
    let current: string | undefined;
    const render = () => {
      const name = !state || !state.connected ? 'offline' : state.kind === 'wifi' ? 'wifi' : 'ethernet';
      if (name !== current) {
        current = name;
        ic.replaceChildren(icon(name, 18));
      }
      const text = state?.connected ? state.ssid || state.iface || state.kind : 'Offline';
      label.textContent = text;
      label.hidden = !cfg.name || !!ctx.panel?.vertical;
      el.classList.toggle('offline', !state?.connected);
      el.title = state?.connected ? `${text}${state.ip ? ' · ' + state.ip : ''}` : 'Offline';
    };
    const poll = () =>
      bridge.call<NetworkState>('network.status').then((s) => {
        state = s;
        render();
      }).catch(() => undefined);
    render();
    poll();
    every(el, 10000, poll);
    return {
      el,
      update(c) {
        cfg = c;
        render();
      },
    };
  },
});
