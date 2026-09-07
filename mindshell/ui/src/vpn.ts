// WireGuard tunnels: NetworkManager connections of type wireguard, read and
// driven through the host's `vpn.*` calls. Shared by the panel widget and its
// popup; the host pushes a `vpn` event whenever NetworkManager reports a change.

import * as bridge from './bridge';
import { store } from './state';
import type { VpnState, VpnTunnel } from './types';

let loading: Promise<VpnState> | undefined;

function apply(s: VpnState): VpnState {
  store.state.vpn = s;
  store.emit('vpn');
  return s;
}

/** Read the tunnel list again (deduplicated while a read is in flight). */
export function vpnRefresh(): Promise<VpnState> {
  if (!loading) {
    loading = bridge
      .call<VpnState>('vpn.list')
      .then(apply)
      .finally(() => {
        loading = undefined;
      });
  }
  return loading;
}

/** The tunnels that are up right now. */
export function activeTunnels(s: VpnState | undefined): VpnTunnel[] {
  return (s?.tunnels ?? []).filter((t) => t.active);
}

/** One line for the bar and the tooltip: the active tunnel, or how many. */
export function vpnSummary(s: VpnState | undefined): string {
  const on = activeTunnels(s);
  if (on.length === 1) return on[0].name;
  if (on.length > 1) return `${on.length} tunnels`;
  return '';
}

async function act(method: string, params: Record<string, unknown>): Promise<VpnState> {
  return apply(await bridge.call<VpnState>(method, params));
}

export const vpnConnect = (id: string): Promise<VpnState> => act('vpn.connect', { id });
export const vpnDisconnect = (id: string): Promise<VpnState> => act('vpn.disconnect', { id });
export const vpnAutoconnect = (id: string, on: boolean): Promise<VpnState> => act('vpn.autoconnect', { id, on });
export const vpnRemove = (id: string): Promise<VpnState> => act('vpn.remove', { id });

/** Open the file chooser and import a wg-quick configuration; `imported` is false when it was dismissed. */
export async function vpnImport(): Promise<VpnState & { imported?: boolean; id?: string }> {
  const r = await bridge.call<VpnState & { imported?: boolean; id?: string }>('vpn.import', {});
  if (r.imported) apply(r);
  return r;
}

/** Middle-click on the widget: drop every active tunnel, or bring up the only one. */
export async function vpnQuickToggle(): Promise<void> {
  const s = store.state.vpn ?? (await vpnRefresh());
  const on = activeTunnels(s);
  if (on.length > 0) {
    for (const t of on) await vpnDisconnect(t.id);
    return;
  }
  const all = s.tunnels ?? [];
  if (all.length === 1) await vpnConnect(all[0].id);
}
