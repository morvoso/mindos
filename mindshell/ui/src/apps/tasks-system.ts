// The pages that go deep on one part of the machine: the drives, the network,
// the sensors, the containers, the services, and the machine itself.

import * as bridge from '../bridge';
import { anchored, every, h } from '../dom';
import { icon } from '../icons';
import { bytes, chart, count, degrees, duration, percent, poll, rate } from '../monitor';
import type { ContainerRow, Containers, DiskInfo, Services, UnitRow } from '../types';
import type { Column } from './tasks-parts';
import { barRow, dataTable, factGrid, panel, state } from './tasks-parts';
import { fmtDate, notice, pageHeader } from './shared';

const TICK = 1000;
/** Containers and units come from other programs; a slower beat is plenty. */
const SLOW_TICK = 5000;

// ------------------------------------------------------------------ storage

export function storagePage(el: HTMLElement): () => void {
  const head = pageHeader('Storage', 'Drives and the space on them');
  const sub = head.querySelector<HTMLElement>('.page-sub')!;
  const io = chart({ points: 90, height: 96, colors: ['var(--accent)', 'var(--warn)'], max: 'auto', fill: true });
  const drives = dataTable<DiskInfo>(
    [
      {
        key: 'device',
        label: 'Drive',
        width: 'minmax(0,1.6fr)',
        cell: (d) => {
          const node = h('span', { class: 'tk-proc' }, h('b', {}, d.device));
          if (d.model) node.append(h('small', {}, d.model));
          return node;
        },
      },
      { key: 'size', label: 'Size', width: '92px', align: 'end', cell: (d) => bytes(d.size, 0) },
      { key: 'kind', label: 'Kind', width: '84px', cell: (d) => (d.removable ? 'Removable' : d.rotational ? 'Hard disk' : 'Solid state') },
      { key: 'read', label: 'Read', width: '96px', align: 'end', cell: (d) => rate(d.readRate) },
      { key: 'write', label: 'Write', width: '96px', align: 'end', cell: (d) => rate(d.writeRate) },
      { key: 'iops', label: 'Ops/s', width: '80px', align: 'end', cell: (d) => count(d.iops) },
      { key: 'util', label: 'Busy', width: '72px', align: 'end', cell: (d) => percent(d.util) },
    ],
    (d) => d.device,
    { empty: 'No drives were found.' },
  );
  const mounts = h('div', { class: 'tk-barrows' });
  const bars = new Map<string, ReturnType<typeof barRow>>();
  const facts = factGrid([], 'tk-facts-wide');

  el.append(
    head,
    panel('Activity', 'Read and written, across every drive', io.el, facts.el),
    panel('Drives', 'The hardware', drives.el),
    panel('Filesystems', 'Where the space went', mounts),
  );

  return poll(el, TICK, { parts: ['storage'] }, (v) => {
    const disks = v.disks ?? [];
    let read = 0;
    let write = 0;
    for (const d of disks) {
      read += d.readRate;
      write += d.writeRate;
    }
    io.push(read, write);
    drives.set(disks);
    const total = (v.filesystems ?? []).reduce((n, f) => n + f.size, 0);
    const free = (v.filesystems ?? []).reduce((n, f) => n + f.avail, 0);
    sub.textContent = `${disks.length} drive${disks.length === 1 ? '' : 's'} · ${bytes(free, 0)} free of ${bytes(total, 0)}`;
    facts.set([
      ['Reading', rate(read)],
      ['Writing', rate(write)],
      ['Operations', `${count(disks.reduce((n, d) => n + d.iops, 0))}/s`],
      ['Busiest drive', disks.reduce((best, d) => (best && best.util >= d.util ? best : d), disks[0])?.device ?? '—'],
    ]);
    const filesystems = v.filesystems ?? [];
    for (const fs of filesystems) {
      let bar = bars.get(fs.mount);
      if (!bar) {
        bar = barRow();
        bars.set(fs.mount, bar);
        mounts.appendChild(bar.el);
      }
      bar.set(
        fs.mount,
        `${fs.device} · ${fs.fstype}${fs.readOnly ? ' · read-only' : ''}`,
        fs.percent / 100,
        `${bytes(fs.used)} used · ${bytes(fs.avail)} free`,
      );
    }
    for (const [mount, bar] of bars) {
      if (!filesystems.some((f) => f.mount === mount)) {
        bar.el.remove();
        bars.delete(mount);
      }
    }
  });
}

// ------------------------------------------------------------------ network

export function networkPage(el: HTMLElement): () => void {
  const head = pageHeader('Network', 'Interfaces and what is moving through them');
  const sub = head.querySelector<HTMLElement>('.page-sub')!;
  const traffic = chart({ points: 90, height: 96, colors: ['var(--accent)', 'var(--warn)'], max: 'auto', fill: true });
  const legend = h(
    'div',
    { class: 'tk-legend' },
    h('span', {}, h('i', { style: { background: 'var(--accent)' } }), 'In'),
    h('span', {}, h('i', { style: { background: 'var(--warn)' } }), 'Out'),
  );
  const facts = factGrid([], 'tk-facts-wide');
  const list = h('div', { class: 'tk-column' });
  const cards = new Map<string, { el: HTMLElement; facts: ReturnType<typeof factGrid>; badge: HTMLElement }>();

  el.append(head, panel('Throughput', 'Across every physical interface', traffic.el, legend, facts.el), panel('Interfaces', undefined, list));

  const glyph = (kind: string) => (kind === 'wifi' ? 'wifi' : kind === 'vpn' ? 'shield' : kind === 'bridge' ? 'layers' : 'ethernet');

  return poll(el, TICK, { parts: ['net'] }, (v) => {
    const nets = v.net ?? [];
    let rx = 0;
    let tx = 0;
    for (const n of nets) {
      if (n.kind === 'bridge' || n.kind === 'virtual') continue;
      rx += n.rxRate;
      tx += n.txRate;
    }
    traffic.push(rx, tx);
    const up = nets.filter((n) => n.state === 'up').length;
    sub.textContent = `${up} of ${nets.length} interface${nets.length === 1 ? '' : 's'} up · ${rate(rx)} in, ${rate(tx)} out`;
    facts.set([
      ['Download', rate(rx)],
      ['Upload', rate(tx)],
      ['Received', bytes(nets.reduce((n, i) => n + i.rx, 0), 0)],
      ['Sent', bytes(nets.reduce((n, i) => n + i.tx, 0), 0)],
    ]);
    for (const n of nets) {
      let card = cards.get(n.iface);
      if (!card) {
        const badge = h('span', { class: 'tk-state idle' }, '');
        const heading = h('header', { class: 'tk-iface-head' }, h('span', { class: 'tk-iface-mark' }, icon(glyph(n.kind), 15)), h('h3', {}, n.iface), badge);
        const grid = factGrid([], 'tk-facts-wide');
        const node = h('section', { class: 'tk-iface' }, heading, grid.el);
        card = { el: node, facts: grid, badge };
        cards.set(n.iface, card);
        list.appendChild(node);
      }
      const live = n.state === 'up';
      card.badge.textContent = live ? 'Up' : n.state;
      card.badge.className = `tk-state ${live ? 'ok' : 'idle'}`;
      card.facts.set([
        ['Type', { wifi: 'Wi-Fi', ethernet: 'Ethernet', vpn: 'VPN', bridge: 'Bridge', virtual: 'Virtual' }[n.kind]],
        ['Address', n.addrs[0] ?? 'None'],
        ['Download', rate(n.rxRate)],
        ['Upload', rate(n.txRate)],
        ['Received', bytes(n.rx, 0)],
        ['Sent', bytes(n.tx, 0)],
        ['Link speed', n.speed ? `${n.speed >= 1000 ? `${n.speed / 1000} Gb/s` : `${n.speed} Mb/s`}` : '—'],
        ['Hardware address', n.mac ?? '—'],
        ['MTU', n.mtu ? String(n.mtu) : '—'],
        ['Errors', count(n.errors)],
      ]);
    }
    for (const [iface, card] of cards) {
      if (!nets.some((n) => n.iface === iface)) {
        card.el.remove();
        cards.delete(iface);
      }
    }
  });
}

// ------------------------------------------------------------------ sensors

export function sensorsPage(el: HTMLElement): () => void {
  const head = pageHeader('Sensors', 'What the machine measures about itself');
  const sub = head.querySelector<HTMLElement>('.page-sub')!;
  const temps = h('div', { class: 'tk-sensors' });
  const fans = h('div', { class: 'tk-sensors' });
  const power = h('div', { class: 'tk-sensors' });
  const tiles = new Map<string, { el: HTMLElement; value: HTMLElement; fill: HTMLElement }>();

  el.append(
    head,
    panel('Temperatures', 'Degrees Celsius', temps),
    panel('Fans', 'Revolutions per minute', fans),
    panel('Power', 'Watts drawn', power),
  );

  /** A sensor tile, made once and written to thereafter. */
  const tile = (host: HTMLElement, key: string, name: string, chip: string) => {
    let node = tiles.get(key);
    if (!node) {
      const value = h('strong', {}, '—');
      const fill = h('i');
      const box = h(
        'div',
        { class: 'tk-sensor' },
        h('div', { class: 'tk-sensor-head' }, h('span', { class: 'tk-sensor-name' }, name), value),
        h('span', { class: 'tk-sensor-chip' }, chip),
        h('div', { class: 'tk-bar' }, fill),
      );
      node = { el: box, value, fill };
      tiles.set(key, node);
      host.appendChild(box);
    }
    return node;
  };

  return poll(el, 2000, { parts: ['sensors'] }, (v) => {
    const s = v.sensors;
    if (!s) return;
    const seen = new Set<string>();
    const hottest = s.temps.reduce<{ value: number; chip: string } | undefined>((best, t) => (!best || t.value > best.value ? t : best), undefined);
    sub.textContent = `${s.temps.length} temperature${s.temps.length === 1 ? '' : 's'}, ${s.fans.length} fan${s.fans.length === 1 ? '' : 's'}${hottest ? ` · hottest ${degrees(hottest.value)} on ${hottest.chip}` : ''}`;
    for (const t of s.temps) {
      const key = `t:${t.chip}:${t.label}`;
      seen.add(key);
      const node = tile(temps, key, t.label, t.chip);
      node.value.textContent = degrees(t.value);
      // 100 °C is where silicon starts protecting itself; the bar is drawn
      // against that, so a warm chip looks warm and a hot one looks hot.
      const share = Math.max(0, Math.min(100, t.value));
      node.fill.style.width = `${share}%`;
      node.fill.style.background = share >= 85 ? 'var(--danger)' : share >= 70 ? 'var(--warn)' : 'var(--accent)';
    }
    for (const f of s.fans) {
      const key = `f:${f.chip}:${f.label}`;
      seen.add(key);
      const node = tile(fans, key, f.label, f.chip);
      node.value.textContent = f.rpm > 0 ? `${Math.round(f.rpm)} rpm` : 'Stopped';
      node.fill.style.width = `${Math.min(100, (f.rpm / 3000) * 100).toFixed(0)}%`;
      node.fill.style.background = 'var(--ok)';
    }
    for (const p of s.power) {
      const key = `p:${p.chip}:${p.label}`;
      seen.add(key);
      const node = tile(power, key, p.label, p.chip);
      node.value.textContent = `${p.watts.toFixed(1)} W`;
      node.fill.style.width = `${Math.min(100, (p.watts / 250) * 100).toFixed(0)}%`;
      node.fill.style.background = 'var(--mind)';
    }
    for (const [key, node] of tiles) {
      if (!seen.has(key)) {
        node.el.remove();
        tiles.delete(key);
      }
    }
    for (const [host, label] of [
      [temps, 'No temperature sensors were found.'],
      [fans, 'No fans report their speed on this machine.'],
      [power, 'No power sensors were found.'],
    ] as [HTMLElement, string][]) {
      if (!host.firstElementChild) host.append(h('p', { class: 'tk-empty' }, label));
    }
  });
}

// --------------------------------------------------------------- containers

export function containersPage(el: HTMLElement): () => void {
  const head = pageHeader('Containers', 'Docker and Podman, if either is installed');
  const sub = head.querySelector<HTMLElement>('.page-sub')!;
  const status = notice();
  const host = h('div', { class: 'tk-column' });
  el.append(head, status.el, host);
  const sections = new Map<string, { el: HTMLElement; table: ReturnType<typeof dataTable<ContainerRow>>; note: HTMLElement }>();

  const engineSection = (engine: 'docker' | 'podman') => {
    let s = sections.get(engine);
    if (!s) {
      const table = dataTable<ContainerRow>(
        [
          {
            key: 'name',
            label: 'Container',
            width: 'minmax(0,1.6fr)',
            cell: (c) => {
              const node = h('span', { class: 'tk-proc' }, h('b', {}, c.name || c.id.slice(0, 12)));
              node.append(h('small', {}, c.image));
              node.title = c.command;
              return node;
            },
          },
          { key: 'state', label: 'State', width: '110px', cell: (c) => state(c.state || 'unknown', c.running ? 'ok' : c.state === 'exited' ? 'idle' : 'warn') },
          { key: 'status', label: 'Status', width: 'minmax(0,1fr)', cell: (c) => c.status },
          { key: 'cpu', label: 'CPU', width: '78px', align: 'end', cell: (c) => (c.cpu != null ? `${c.cpu.toFixed(1)}%` : '—') },
          { key: 'mem', label: 'Memory', width: '130px', align: 'end', cell: (c) => c.mem ?? '—' },
          { key: 'ports', label: 'Ports', width: 'minmax(0,1fr)', cell: (c) => (typeof c.ports === 'number' ? (c.ports ? `${c.ports} published` : '') : c.ports) },
        ],
        (c) => c.id,
        { empty: 'No containers.' },
      );
      const note = h('p', { class: 'tk-empty', hidden: true });
      const node = panel(engine === 'docker' ? 'Docker' : 'Podman', undefined, note, table.el);
      s = { el: node, table, note };
      sections.set(engine, s);
      host.appendChild(node);
    }
    return s;
  };

  let busy = false;
  const refresh = async () => {
    if (busy || el.offsetParent === null) return;
    busy = true;
    try {
      const v = await bridge.call<Containers>('system.containers', { stats: true });
      if (!el.isConnected) return;
      anchored(el, () => {
        let running = 0;
        let total = 0;
        let installed = 0;
        for (const engine of ['docker', 'podman'] as const) {
          const info = v[engine];
          if (!info.available) {
            sections.get(engine)?.el.remove();
            sections.delete(engine);
            continue;
          }
          installed += 1;
          const s = engineSection(engine);
          s.table.set(info.containers);
          running += info.containers.filter((c) => c.running).length;
          total += info.containers.length;
          s.note.hidden = info.running && !info.error;
          s.note.textContent = info.error ?? (info.running ? '' : `${engine} is installed, but its service is not running.`);
        }
        sub.textContent = installed === 0 ? 'Neither Docker nor Podman is installed.' : `${running} running of ${total} container${total === 1 ? '' : 's'}`;
        if (installed === 0 && !host.firstElementChild) {
          host.append(
            h(
              'p',
              { class: 'tk-empty' },
              icon('docker', 15),
              ' No container engine is installed. Install Docker or Podman and its containers will appear here.',
            ),
          );
        }
      });
    } catch (e) {
      if (el.isConnected) status.show(bridge.reason(e), 'error');
    } finally {
      busy = false;
    }
  };

  const stop = every(el, SLOW_TICK, () => void refresh());
  return () => {
    stop();
    status.clear();
  };
}

// ----------------------------------------------------------------- services

export function servicesPage(el: HTMLElement): () => void {
  const head = pageHeader('Services', 'The units systemd is running');
  const sub = head.querySelector<HTMLElement>('.page-sub')!;
  const status = notice();
  const failed = dataTable<UnitRow>(unitColumns(), (u) => `f:${u.unit}`, { empty: 'Nothing has failed.' });
  const system = dataTable<UnitRow>(unitColumns(), (u) => `s:${u.unit}`, { empty: 'No system services are running.' });
  const user = dataTable<UnitRow>(unitColumns(), (u) => `u:${u.unit}`, { empty: 'No user services are running.' });
  const failedPanel = panel('Failed', 'Units that could not start, or stopped badly', failed.el);
  el.append(head, status.el, failedPanel, panel('System services', 'Running for the machine', system.el), panel('Your services', 'Running for this login', user.el));

  let busy = false;
  const refresh = async () => {
    if (busy || el.offsetParent === null) return;
    busy = true;
    try {
      const v = await bridge.call<Services>('system.services');
      if (!el.isConnected) return;
      if (!v.available) {
        sub.textContent = 'systemd is not present on this machine.';
        return;
      }
      const bad = [...v.system.failed, ...v.user.failed];
      anchored(el, () => {
        failedPanel.hidden = bad.length === 0;
        failed.set(bad);
        system.set(v.system.units);
        user.set(v.user.units);
        sub.textContent = `${v.system.running} system · ${v.user.running} yours${bad.length ? ` · ${bad.length} failed` : ''}`;
      });
    } catch (e) {
      if (el.isConnected) status.show(bridge.reason(e), 'error');
    } finally {
      busy = false;
    }
  };

  const stop = every(el, SLOW_TICK, () => void refresh());
  return () => {
    stop();
    status.clear();
  };
}

function unitColumns(): Column<UnitRow>[] {
  return [
    { key: 'unit', label: 'Unit', width: 'minmax(0,1.4fr)', cell: (u: UnitRow) => u.unit },
    {
      key: 'active',
      label: 'State',
      width: '120px',
      cell: (u: UnitRow) => state(u.sub || u.active, u.active === 'failed' ? 'danger' : u.sub === 'running' ? 'ok' : 'idle'),
    },
    { key: 'description', label: 'Description', width: 'minmax(0,2fr)', cell: (u: UnitRow) => u.description },
  ];
}

// ------------------------------------------------------------------- system

export function systemPage(el: HTMLElement): () => void {
  const head = pageHeader('System', 'This machine');
  const machine = factGrid([], 'tk-facts-wide');
  const software = factGrid([], 'tk-facts-wide');
  const hardware = factGrid([], 'tk-facts-wide');
  el.append(
    head,
    panel('Machine', undefined, machine.el),
    panel('Hardware', undefined, hardware.el),
    panel('Software', undefined, software.el),
  );

  return poll(el, 5000, { parts: ['host'] }, (v) => {
    const host = v.host;
    if (!host) return;
    machine.set([
      ['Name', host.hostname],
      ['Signed in as', host.user],
      ['Uptime', duration(host.uptime)],
      ['Booted', fmtDate(host.boot)],
      ['Session', host.session ?? '—'],
      ['Processes', count(v.cpu.procs)],
    ]);
    hardware.set([
      ['Processor', v.cpu.model],
      ['Cores', `${v.cpu.cores} cores · ${v.cpu.threads} threads`],
      ['Memory', bytes(v.memory.total, 0)],
      ['Graphics', v.gpus.map((g) => g.name).join(', ') || '—'],
      ['Motherboard', host.board ?? host.product ?? '—'],
      ['Firmware', host.bios ?? '—'],
    ]);
    software.set([
      ['Operating system', host.os],
      ['Kernel', `${host.kernel ?? '—'} (${host.arch})`],
      ['Desktop shell', `mindshell ${host.shell}`],
      ['Packages installed', host.packages != null ? count(host.packages) : '—'],
    ]);
  });
}
