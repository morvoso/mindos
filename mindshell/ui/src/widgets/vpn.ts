// The VPN indicator: a shield that lights up while a WireGuard tunnel is up,
// with the tunnel's name beside it. Click for the tunnel list, middle-click
// to drop the active tunnel (or bring up the only one).

import { every, h } from '../dom';
import { icon } from '../icons';
import { registerWidget } from './registry';
import { panelItem } from './common';
import { activeTunnels, vpnQuickToggle, vpnRefresh, vpnSummary } from '../vpn';

registerWidget({
  type: 'vpn',
  name: 'VPN',
  description: 'WireGuard tunnels. The shield lights up while one is connected; click to connect, disconnect or import a configuration.',
  icon: 'shield',
  containers: ['panel'],
  defaults: { name: true, hideWhenNone: false },
  settings: {
    name: { label: 'Show the tunnel name', type: 'boolean', help: 'The name of the connected tunnel, or how many are up' },
    hideWhenNone: { label: 'Hide when no tunnel is set up', type: 'boolean', help: 'The icon stays hidden until a WireGuard configuration has been imported' },
  },
  create(ctx) {
    const el = panelItem(ctx, 'w-vpn', 'VPN');
    const ic = h('span', { class: 'w-ic' }, icon('shield', 18));
    const label = h('span', { class: 'w-label' });
    el.append(ic, label);
    let cfg = ctx.config;
    let busy = false;
    const render = () => {
      const s = ctx.store.state.vpn;
      const on = activeTunnels(s);
      const total = s?.tunnels?.length ?? 0;
      const activating = (s?.tunnels ?? []).some((t) => t.activating);
      const text = vpnSummary(s);
      label.textContent = text;
      label.hidden = !cfg.name || !text || !!ctx.panel?.vertical;
      el.classList.toggle('on', on.length > 0);
      el.classList.toggle('busy', busy || activating);
      el.classList.toggle('hidden', !!cfg.hideWhenNone && total === 0 && !!s);
      el.title = !s
        ? 'VPN'
        : s.available === false
          ? 'VPN · NetworkManager is not running'
          : on.length > 0
            ? `VPN · ${on.map((t) => t.name + (t.endpoint ? ' → ' + t.endpoint : '')).join(', ')}`
            : total === 0
              ? 'VPN · no tunnels set up'
              : `VPN · off (${total} tunnel${total === 1 ? '' : 's'})`;
    };
    render();
    void vpnRefresh().catch(() => undefined);
    ctx.store.bind(el, 'vpn', render);
    ctx.store.bind(el, 'popups', () => el.classList.toggle('open', ctx.store.popups.has('vpn')));
    // NetworkManager's change feed drives the list; this is the safety net.
    every(el, 60000, () => void vpnRefresh().catch(() => undefined));
    el.addEventListener('click', () => ctx.togglePopup('vpn', {}, { anchor: ctx.anchorOf(el) }));
    el.addEventListener('auxclick', (e) => {
      if (e.button !== 1 || busy) return;
      busy = true;
      render();
      vpnQuickToggle()
        .catch(() => undefined)
        .finally(() => {
          busy = false;
          render();
        });
    });
    return {
      el,
      update(c) {
        cfg = c;
        render();
      },
    };
  },
});
