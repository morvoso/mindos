// The tunnel list: every WireGuard tunnel NetworkManager knows, a switch for
// each, and a row's details (interface, address, endpoint, start-up
// behaviour, remove) on click. Import brings in a wg-quick file.

import { reason } from '../bridge';
import { h } from '../dom';
import { icon } from '../icons';
import type { VpnTunnel } from '../types';
import { vpnAutoconnect, vpnConnect, vpnDisconnect, vpnImport, vpnRefresh, vpnRemove } from '../vpn';
import { armable } from './shared';
import type { PopupContent, PopupCtx } from './shared';

export function vpnPopup(ctx: PopupCtx): PopupContent {
  const store = ctx.store;
  const list = h('div', { class: 'vpn-list' });
  const empty = h('div', { class: 'vpn-empty', hidden: true });
  const err = h('div', { class: 'pop-hint danger', hidden: true });
  const importBtn = h('button', { class: 'btn small', title: 'Import a WireGuard configuration file' }, icon('download', 14), 'Import…');
  const open = new Set<string>();
  let pending = 0;

  const fail = (what: string, e: unknown) => {
    err.textContent = `${what}: ${reason(e)}`.replace(/: (Could not [^:]+): /, ': $1: ');
    err.hidden = false;
    ctx.relayout();
  };
  const run = (p: Promise<unknown>, what: string) => {
    pending++;
    list.classList.toggle('busy', pending > 0);
    err.hidden = true;
    return p
      .then(() => {
        err.hidden = true;
      })
      .catch((e) => fail(what, e))
      .finally(() => {
        pending--;
        list.classList.toggle('busy', pending > 0);
        ctx.relayout();
      });
  };

  const row = (t: VpnTunnel): HTMLElement => {
    const sw = h('input', { type: 'checkbox' }) as HTMLInputElement;
    sw.checked = t.active;
    sw.addEventListener('change', () => void run(sw.checked ? vpnConnect(t.id) : vpnDisconnect(t.id), sw.checked ? 'Could not connect' : 'Could not disconnect'));
    const sub = [t.address, t.endpoint].filter(Boolean).join(' · ') || (t.iface ? `Interface ${t.iface}` : 'No address');
    const main = h(
      'div',
      { class: 'vpn-main' },
      h('span', { class: 'vpn-ic' }, icon('shield', 20)),
      h('div', { class: 'vpn-text' }, h('span', { class: 'vpn-name' }, t.name), h('span', { class: 'vpn-blurb' }, t.activating ? 'Connecting…' : t.active ? `Connected · ${sub}` : sub)),
      h('label', { class: 'switch', title: t.active ? 'Disconnect' : 'Connect', onclick: (e: Event) => e.stopPropagation() }, sw, h('i')),
    );
    const auto = h('input', { type: 'checkbox' }) as HTMLInputElement;
    auto.checked = t.autoconnect;
    auto.addEventListener('change', () => void run(vpnAutoconnect(t.id, auto.checked), 'Could not change the tunnel'));
    const removeLabel = h('span', { class: 'sr' }, 'Remove');
    const remove = h('button', { class: 'tool danger', title: 'Remove this tunnel' }, icon('trash', 15), removeLabel);
    armable(remove, removeLabel, 'Remove', 'Remove?', () => void run(vpnRemove(t.id), 'Could not remove the tunnel'));
    const kv = (k: string, v: string | undefined) => (v ? h('div', { class: 'vpn-kv' }, h('span', {}, k), h('span', { class: 'mono' }, v)) : null);
    const detail = h(
      'div',
      { class: 'vpn-detail' },
      kv('Interface', t.iface),
      kv('Address', t.address),
      kv('Endpoint', t.endpoint),
      kv('Peers', t.peers ? String(t.peers) : undefined),
      h('div', { class: 'vpn-kv vpn-opt' }, h('span', {}, 'Connect at start-up'), h('label', { class: 'switch' }, auto, h('i'))),
      h('div', { class: 'vpn-kv' }, h('span', {}, 'Forget this tunnel and its keys'), remove),
    );
    const el = h('div', { class: `vpn-row${t.active ? ' on' : ''}${t.activating ? ' activating' : ''}${open.has(t.id) ? ' open' : ''}` }, main, detail);
    main.addEventListener('click', () => {
      if (open.has(t.id)) open.delete(t.id);
      else open.add(t.id);
      el.classList.toggle('open', open.has(t.id));
      ctx.relayout();
    });
    return el;
  };

  const render = () => {
    const s = store.state.vpn;
    const tunnels = s?.tunnels ?? [];
    list.replaceChildren(...tunnels.map(row));
    if (!s) empty.textContent = 'Looking for tunnels…';
    else if (s.available === false) empty.textContent = 'NetworkManager is not running, so tunnels cannot be managed here.';
    else if (tunnels.length === 0) empty.textContent = 'No tunnels yet. Import the configuration file your VPN provider gave you (a wg-quick .conf) to add one.';
    empty.hidden = !!s && s.available !== false && tunnels.length > 0;
    importBtn.disabled = !!s && s.available === false;
    ctx.relayout();
  };

  importBtn.addEventListener('click', () => {
    void run(
      vpnImport().then((r) => {
        if (r.imported && r.id) open.add(r.id);
      }),
      'Could not import',
    );
  });

  render();
  void vpnRefresh().catch((e) => fail('Could not read the tunnels', e));
  store.bind(list, 'vpn', render);
  const head = h('div', { class: 'pop-head' }, h('span', { class: 'pop-title' }, 'WIREGUARD'), h('span', { class: 'strip-gap' }), importBtn);
  const foot = h('div', { class: 'pop-hint' }, 'Tunnels are kept by NetworkManager and stay up when the shell restarts.');
  const el = h('div', { class: 'pop-body vpn' }, head, list, empty, err, foot);
  return { el, w: 360 };
}
