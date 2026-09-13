// The machine, invented: what the Task Manager and the desktop readout see
// when there is no host behind the bridge (`npm run dev`, `npm run shot`).
//
// Everything here breathes. The processor wanders, the drives and the network
// carry traffic, a container starts and stops, and the process table's places
// change hands — otherwise a graph in the browser is a straight line and a
// layout mistake in a live number never shows up until it is on a real machine.

import type {
  ContainerRow,
  Containers,
  CpuInfo,
  DiskInfo,
  FilesystemInfo,
  GpuInfo,
  HostInfo,
  MemoryInfo,
  NetInfo,
  Overview,
  ProcessDetail,
  ProcessRow,
  ProcessTable,
  SensorInfo,
  Services,
  UnitRow,
} from './types';

const GIB = 1073741824;
const MIB = 1048576;

/** A smooth pseudo-random wander in 0..1, different for every `seed`. */
function wave(t: number, seed: number): number {
  const v = Math.sin(t / (3 + seed * 0.7) + seed) * 0.5 + Math.sin(t / (11 + seed) + seed * 2) * 0.3 + Math.sin(t * 1.9 + seed) * 0.2;
  return (v + 1) / 2;
}

const clamp = (v: number, lo: number, hi: number) => Math.max(lo, Math.min(hi, v));

interface Proc {
  pid: number;
  ppid: number;
  name: string;
  cmd: string;
  user: string;
  uid: number;
  rss: number;
  threads: number;
  /** How hard this one works, 0..1, before the wander is applied. */
  weight: number;
  wine?: boolean;
  nice?: number;
}

const PROCS: Proc[] = [
  { pid: 1, ppid: 0, name: 'systemd', cmd: '/sbin/init', user: 'root', uid: 0, rss: 14 * MIB, threads: 1, weight: 0.01 },
  { pid: 412, ppid: 1, name: 'systemd-journald', cmd: '/usr/lib/systemd/systemd-journald', user: 'root', uid: 0, rss: 68 * MIB, threads: 1, weight: 0.02 },
  { pid: 640, ppid: 1, name: 'NetworkManager', cmd: '/usr/bin/NetworkManager --no-daemon', user: 'root', uid: 0, rss: 22 * MIB, threads: 3, weight: 0.01 },
  { pid: 712, ppid: 1, name: 'pipewire', cmd: '/usr/bin/pipewire', user: 'morvoso', uid: 1000, rss: 31 * MIB, threads: 4, weight: 0.06, nice: -11 },
  { pid: 718, ppid: 1, name: 'wireplumber', cmd: '/usr/bin/wireplumber', user: 'morvoso', uid: 1000, rss: 26 * MIB, threads: 4, weight: 0.03, nice: -11 },
  { pid: 901, ppid: 1, name: 'mindwm', cmd: 'mindwm', user: 'morvoso', uid: 1000, rss: 214 * MIB, threads: 9, weight: 0.22 },
  { pid: 934, ppid: 901, name: 'mindshell', cmd: 'mindshell', user: 'morvoso', uid: 1000, rss: 186 * MIB, threads: 12, weight: 0.14 },
  { pid: 936, ppid: 934, name: 'WebKitWebProcess', cmd: '/usr/lib/webkit2gtk-4.1/WebKitWebProcess 21', user: 'morvoso', uid: 1000, rss: 412 * MIB, threads: 16, weight: 0.18 },
  { pid: 1020, ppid: 1, name: 'mindd', cmd: '/usr/bin/mindd --model qwen3:14b', user: 'morvoso', uid: 1000, rss: 8.9 * GIB, threads: 22, weight: 0.35 },
  { pid: 1211, ppid: 1, name: 'steam', cmd: '/usr/lib/steam/steam', user: 'morvoso', uid: 1000, rss: 640 * MIB, threads: 41, weight: 0.09 },
  { pid: 1288, ppid: 1211, name: 'steamwebhelper', cmd: 'steamwebhelper --type=renderer', user: 'morvoso', uid: 1000, rss: 1.1 * GIB, threads: 33, weight: 0.12 },
  { pid: 2044, ppid: 1211, name: 'Hades2.exe', cmd: 'Z:\\games\\Hades II\\Hades2.exe', user: 'morvoso', uid: 1000, rss: 5.4 * GIB, threads: 28, weight: 0.92, wine: true },
  { pid: 2071, ppid: 2044, name: 'wineserver', cmd: '/usr/bin/wineserver', user: 'morvoso', uid: 1000, rss: 96 * MIB, threads: 2, weight: 0.11, wine: true },
  { pid: 2310, ppid: 934, name: 'firefox', cmd: '/usr/lib/firefox/firefox', user: 'morvoso', uid: 1000, rss: 1.6 * GIB, threads: 96, weight: 0.28 },
  { pid: 2340, ppid: 2310, name: 'Isolated Web Co', cmd: '/usr/lib/firefox/firefox -contentproc -childID 7', user: 'morvoso', uid: 1000, rss: 780 * MIB, threads: 24, weight: 0.21 },
  { pid: 2612, ppid: 934, name: 'foot', cmd: 'foot', user: 'morvoso', uid: 1000, rss: 42 * MIB, threads: 5, weight: 0.02 },
  { pid: 2615, ppid: 2612, name: 'zsh', cmd: '-zsh', user: 'morvoso', uid: 1000, rss: 9 * MIB, threads: 1, weight: 0.01 },
  { pid: 3001, ppid: 1, name: 'dockerd', cmd: '/usr/bin/dockerd -H fd://', user: 'root', uid: 0, rss: 128 * MIB, threads: 18, weight: 0.03 },
  { pid: 3110, ppid: 3001, name: 'postgres', cmd: 'postgres -D /var/lib/postgresql/data', user: 'root', uid: 0, rss: 220 * MIB, threads: 8, weight: 0.07 },
  { pid: 3402, ppid: 1, name: 'sshd', cmd: 'sshd: /usr/bin/sshd -D', user: 'root', uid: 0, rss: 11 * MIB, threads: 1, weight: 0.005 },
  { pid: 3688, ppid: 1, name: 'cupsd', cmd: '/usr/bin/cupsd -l', user: 'root', uid: 0, rss: 18 * MIB, threads: 2, weight: 0.005 },
  { pid: 4102, ppid: 934, name: 'nautilus', cmd: '/usr/bin/nautilus --gapplication-service', user: 'morvoso', uid: 1000, rss: 210 * MIB, threads: 11, weight: 0.02 },
  { pid: 4400, ppid: 1, name: 'kworker/u64:3', cmd: '', user: 'root', uid: 0, rss: 0, threads: 1, weight: 0.04 },
  { pid: 4401, ppid: 1, name: 'kworker/2:1', cmd: '', user: 'root', uid: 0, rss: 0, threads: 1, weight: 0.02 },
  { pid: 4512, ppid: 1, name: 'nvidia-persistenced', cmd: '/usr/bin/nvidia-persistenced', user: 'root', uid: 0, rss: 6 * MIB, threads: 1, weight: 0.005 },
];

const CONTAINERS: (Omit<ContainerRow, 'cpu' | 'mem' | 'memPercent' | 'net' | 'block' | 'pids'> & { load: number })[] = [
  { engine: 'docker', id: 'f3a91c2b77e4', name: 'mind-postgres', image: 'postgres:16-alpine', command: 'docker-entrypoint.sh postgres', status: 'Up 6 days', state: 'running', running: true, ports: '5432/tcp → 127.0.0.1:5432', created: '6 days ago', size: '0B (virtual 243MB)', load: 0.4 },
  { engine: 'docker', id: '9c1de0447a15', name: 'grafana', image: 'grafana/grafana:11.2.0', command: '/run.sh', status: 'Up 6 days', state: 'running', running: true, ports: '3000/tcp → 0.0.0.0:3000', created: '6 days ago', size: '32.1kB (virtual 412MB)', load: 0.15 },
  { engine: 'docker', id: '4b8e5527c930', name: 'redis-cache', image: 'redis:7-alpine', command: 'redis-server', status: 'Exited (0) 2 hours ago', state: 'exited', running: false, ports: '', created: '3 weeks ago', size: '0B (virtual 41MB)', load: 0 },
  { engine: 'podman', id: 'a71f30c9db82', name: 'archlinux-build', image: 'docker.io/library/archlinux:latest', command: '/bin/bash', status: 'Up 40 minutes', state: 'running', running: true, ports: '', created: '40 minutes ago', load: 0.85 },
];

const SYSTEM_UNITS: UnitRow[] = [
  { unit: 'NetworkManager.service', load: 'loaded', active: 'active', sub: 'running', description: 'Network Manager' },
  { unit: 'bluetooth.service', load: 'loaded', active: 'active', sub: 'running', description: 'Bluetooth service' },
  { unit: 'docker.service', load: 'loaded', active: 'active', sub: 'running', description: 'Docker Application Container Engine' },
  { unit: 'greetd.service', load: 'loaded', active: 'active', sub: 'running', description: 'Greeter daemon' },
  { unit: 'nvidia-persistenced.service', load: 'loaded', active: 'active', sub: 'running', description: 'NVIDIA Persistence Daemon' },
  { unit: 'polkit.service', load: 'loaded', active: 'active', sub: 'running', description: 'Authorization Manager' },
  { unit: 'sshd.service', load: 'loaded', active: 'active', sub: 'running', description: 'OpenSSH Daemon' },
  { unit: 'systemd-journald.service', load: 'loaded', active: 'active', sub: 'running', description: 'Journal Service' },
  { unit: 'systemd-timesyncd.service', load: 'loaded', active: 'active', sub: 'running', description: 'Network Time Synchronization' },
  { unit: 'systemd-udevd.service', load: 'loaded', active: 'active', sub: 'running', description: 'Rule-based Manager for Device Events and Files' },
  { unit: 'cups.service', load: 'loaded', active: 'failed', sub: 'failed', description: 'CUPS Scheduler' },
  { unit: 'fstrim.timer', load: 'loaded', active: 'active', sub: 'waiting', description: 'Discard unused filesystem blocks once a week' },
];

const USER_UNITS: UnitRow[] = [
  { unit: 'mindos-shell.service', load: 'loaded', active: 'active', sub: 'running', description: 'The MindOS desktop shell' },
  { unit: 'mindd.service', load: 'loaded', active: 'active', sub: 'running', description: 'The Mind daemon' },
  { unit: 'pipewire.service', load: 'loaded', active: 'active', sub: 'running', description: 'PipeWire Multimedia Service' },
  { unit: 'wireplumber.service', load: 'loaded', active: 'active', sub: 'running', description: 'Multimedia Service Session Manager' },
  { unit: 'xdg-desktop-portal.service', load: 'loaded', active: 'active', sub: 'running', description: 'Portal service' },
  { unit: 'mindos-session.target', load: 'loaded', active: 'active', sub: 'active', description: 'MindOS session' },
];

export interface SystemMock {
  overview(params: Record<string, unknown>): Overview;
  processes(params: Record<string, unknown>): ProcessTable;
  process(params: Record<string, unknown>): ProcessDetail;
  kill(params: Record<string, unknown>): { pid: number };
  containers(params: Record<string, unknown>): Containers;
  services(): Services;
}

export function systemMock(t0: number): SystemMock {
  const CORES = 16;
  const THREADS = 32;
  /** Processes the mock has been asked to end, so the button does something. */
  const ended = new Set<number>();
  const boot = Math.round(t0 / 1000) - (3600 * 29 + 1140);

  const secs = () => (Date.now() - t0) / 1000;
  const live = () => PROCS.filter((p) => !ended.has(p.pid));

  /** Every process's share of the machine right now, keyed by pid. */
  const loads = (t: number): Map<number, number> => {
    const map = new Map<number, number>();
    for (const p of live()) map.set(p.pid, clamp(p.weight * (0.45 + wave(t, p.pid % 17) * 1.5) * 100, 0, 780));
    return map;
  };

  const cpu = (t: number): CpuInfo => {
    const perCore = Array.from({ length: THREADS }, (_, i) => clamp(8 + wave(t, i) * 70 + (i % 8 === 0 ? 12 : 0), 0, 100));
    const usage = perCore.reduce((a, b) => a + b, 0) / perCore.length;
    return {
      model: 'AMD Ryzen 9 7950X 16-Core Processor',
      vendor: 'AuthenticAMD',
      cores: CORES,
      threads: THREADS,
      usage,
      perCore,
      kinds: { user: usage * 0.72, system: usage * 0.2, iowait: usage * 0.05, irq: usage * 0.03 },
      freq: perCore.map((c) => 3400 + c * 21),
      freqAvg: 3400 + usage * 21,
      freqMax: 5700,
      governor: 'schedutil',
      driver: 'amd-pstate-epp',
      epp: 'balance_performance',
      temp: 48 + usage * 0.34,
      tempLabel: 'Tctl',
      load: [1.9 + wave(t, 3), 1.4 + wave(t, 5), 1.1],
      procs: live().length + 190,
      running: 2 + Math.round(wave(t, 7) * 4),
      ctxtRate: 18000 + wave(t, 9) * 9000,
      intrRate: 12000 + wave(t, 11) * 6000,
      forkRate: 4 + wave(t, 13) * 30,
    };
  };

  const memory = (t: number): MemoryInfo => {
    const total = 64 * GIB;
    const used = 21 * GIB + wave(t, 2) * 4 * GIB;
    const cached = 17 * GIB + wave(t, 4) * GIB;
    return {
      total,
      used,
      available: total - used - cached * 0.2,
      free: total - used - cached,
      buffers: 640 * MIB,
      cached,
      shared: 1.2 * GIB,
      dirty: 12 * MIB,
      slab: 1.4 * GIB,
      kernel: 820 * MIB,
      swapTotal: 8 * GIB,
      swapUsed: 512 * MIB + wave(t, 6) * 128 * MIB,
      swapFree: 8 * GIB - 512 * MIB,
      zram: [{ name: 'zram0', size: 8 * GIB, stored: 1.4 * GIB, compressed: 460 * MIB, used: 470 * MIB, algorithm: 'zstd' }],
      swaps: [{ name: '/dev/zram0', kind: 'partition', size: 8 * GIB, used: 512 * MIB, priority: 100 }],
    };
  };

  const gpus = (t: number): GpuInfo[] => {
    const util = clamp(30 + wave(t, 8) * 65, 0, 100);
    return [
      {
        name: 'NVIDIA GeForce RTX 4090',
        vendor: 'NVIDIA',
        util,
        memUtil: 38 + wave(t, 10) * 30,
        mem: 6200 + wave(t, 12) * 3400,
        memTotal: 24564,
        temp: 52 + util * 0.24,
        power: 90 + util * 3.4,
        powerLimit: 450,
        clock: 1900 + util * 7,
        memClock: 10501,
        fan: 1200 + util * 14,
        fanPercent: 28 + util * 0.4,
        driver: '565.77',
      },
      { name: 'AMD Raphael', vendor: 'AMD', util: null, mem: null, memTotal: null, temp: 44 + wave(t, 14) * 6, driver: 'amdgpu' },
    ];
  };

  const disks = (t: number): DiskInfo[] => [
    { device: 'nvme0n1', model: 'Samsung SSD 990 PRO 2TB', size: 2 * 1000 * 1000 * 1000 * 1000, rotational: false, removable: false, scheduler: 'none', readRate: wave(t, 15) * 420 * MIB, writeRate: wave(t, 16) * 180 * MIB, util: wave(t, 15) * 64, iops: Math.round(wave(t, 15) * 9000) },
    { device: 'nvme1n1', model: 'WD_BLACK SN850X 4TB', size: 4 * 1000 * 1000 * 1000 * 1000, rotational: false, removable: false, scheduler: 'none', readRate: wave(t, 17) * 140 * MIB, writeRate: wave(t, 18) * 60 * MIB, util: wave(t, 17) * 22, iops: Math.round(wave(t, 17) * 2600) },
    { device: 'sda', model: 'ST8000DM004-2U91', size: 8 * 1000 * 1000 * 1000 * 1000, rotational: true, removable: false, scheduler: 'bfq', readRate: wave(t, 19) * 30 * MIB, writeRate: wave(t, 20) * 12 * MIB, util: wave(t, 19) * 14, iops: Math.round(wave(t, 19) * 240) },
  ];

  const filesystems = (): FilesystemInfo[] => [
    { device: '/dev/nvme0n1p2', mount: '/', fstype: 'btrfs', readOnly: false, size: 1.86 * 1000 * GIB, used: 612 * GIB, avail: 1.25 * 1000 * GIB, percent: 32.9 },
    { device: '/dev/nvme0n1p1', mount: '/boot', fstype: 'vfat', readOnly: false, size: 1023 * MIB, used: 412 * MIB, avail: 611 * MIB, percent: 40.3 },
    { device: '/dev/nvme1n1p1', mount: '/games', fstype: 'ext4', readOnly: false, size: 3.63 * 1000 * GIB, used: 3.1 * 1000 * GIB, avail: 530 * GIB, percent: 85.4 },
    { device: '/dev/sda1', mount: '/mnt/archive', fstype: 'xfs', readOnly: false, size: 7.27 * 1000 * GIB, used: 2.4 * 1000 * GIB, avail: 4.87 * 1000 * GIB, percent: 33.0 },
  ];

  const net = (t: number): NetInfo[] => [
    { iface: 'enp5s0', kind: 'ethernet', state: 'up', mac: '3c:7c:3f:11:d9:04', mtu: 1500, speed: 2500, addrs: ['192.168.1.24/24', 'fd00::24/64'], rx: 412 * GIB, tx: 88 * GIB, rxRate: wave(t, 21) * 62 * MIB, txRate: wave(t, 22) * 9 * MIB, errors: 0 },
    { iface: 'wlan0', kind: 'wifi', state: 'down', mac: '9c:b6:d0:8a:2e:71', mtu: 1500, speed: null, addrs: [], rx: 0, tx: 0, rxRate: 0, txRate: 0, errors: 0 },
    { iface: 'wg0', kind: 'vpn', state: 'up', mac: null, mtu: 1420, speed: null, addrs: ['10.66.0.2/24'], rx: 2.1 * GIB, tx: 640 * MIB, rxRate: wave(t, 23) * 2 * MIB, txRate: wave(t, 24) * 1.2 * MIB, errors: 0 },
    { iface: 'docker0', kind: 'bridge', state: 'up', mac: '02:42:6c:1f:3b:aa', mtu: 1500, speed: null, addrs: ['172.17.0.1/16'], rx: 118 * MIB, tx: 402 * MIB, rxRate: wave(t, 25) * 400 * 1024, txRate: wave(t, 26) * 900 * 1024, errors: 0 },
  ];

  const sensors = (t: number): SensorInfo => ({
    temps: [
      { chip: 'k10temp', label: 'Tctl', value: 48 + wave(t, 27) * 34 },
      { chip: 'k10temp', label: 'Tccd1', value: 45 + wave(t, 28) * 28 },
      { chip: 'nvme', label: 'Composite', value: 41 + wave(t, 29) * 18 },
      { chip: 'nouveau', label: 'GPU core', value: 52 + wave(t, 8) * 26 },
      { chip: 'acpitz', label: 'Chassis', value: 34 + wave(t, 30) * 6 },
      { chip: 'nct6798', label: 'VRM', value: 44 + wave(t, 31) * 22 },
    ],
    fans: [
      { chip: 'nct6798', label: 'CPU fan', rpm: 900 + wave(t, 32) * 1100 },
      { chip: 'nct6798', label: 'Front intake', rpm: 700 + wave(t, 33) * 500 },
      { chip: 'nct6798', label: 'Rear exhaust', rpm: 760 + wave(t, 34) * 460 },
      { chip: 'nvidia', label: 'GPU fan', rpm: 1200 + wave(t, 8) * 1400 },
    ],
    power: [
      { chip: 'amd_energy', label: 'Package', watts: 38 + wave(t, 35) * 110 },
      { chip: 'nvidia', label: 'Board', watts: 90 + wave(t, 8) * 300 },
    ],
  });

  const host = (): HostInfo => ({
    hostname: 'mindos-dev',
    os: 'MindOS Rolling',
    osId: 'mindos',
    kernel: '6.17.4-arch1-1',
    arch: 'x86_64',
    uptime: Math.round(Date.now() / 1000) - boot,
    boot,
    product: 'System Product Name',
    board: 'ASUSTeK COMPUTER INC. ROG STRIX X670E-E GAMING WIFI',
    bios: '2402',
    packages: 1642,
    session: 'wayland',
    shell: '0.1.0',
    user: 'morvoso',
  });

  const rows = (t: number): ProcessRow[] => {
    const load = loads(t);
    return live().map((p) => ({
      pid: p.pid,
      ppid: p.ppid,
      name: p.name,
      state: p.pid % 7 === 0 ? 'S' : 'R',
      cpu: load.get(p.pid) ?? 0,
      rss: p.rss * (0.95 + wave(t, p.pid % 13) * 0.1),
      vsize: p.rss * 3.4 + 512 * MIB,
      threads: p.threads,
      prio: 20,
      nice: p.nice ?? 0,
      started: boot + p.pid,
      uid: p.uid,
      user: p.user,
      cmd: p.cmd,
      wine: !!p.wine,
      own: p.uid === 1000,
      readRate: wave(t, p.pid % 19) * 2 * MIB * p.weight,
      writeRate: wave(t, p.pid % 23) * MIB * p.weight,
    }));
  };

  return {
    overview(params) {
      const t = secs();
      const parts = Array.isArray(params.parts) ? (params.parts as string[]) : null;
      const want = (name: string) => !parts || parts.includes(name);
      const top = Number(params.top) || 0;
      const v: Overview = { at: Date.now() / 1000, cpu: cpu(t), memory: memory(t), gpus: gpus(t) };
      if (want('storage')) {
        v.disks = disks(t);
        v.filesystems = filesystems();
      }
      if (want('net')) v.net = net(t);
      if (want('sensors')) v.sensors = sensors(t);
      if (want('host')) v.host = host();
      if (want('containers')) {
        v.containers = {
          docker: { available: true, running: 2, total: 3 },
          podman: { available: true, running: 1, total: 1 },
        };
      }
      if (top > 0) v.top = rows(t).sort((a, b) => b.cpu - a.cpu).slice(0, top);
      return v;
    },

    processes(params) {
      const t = secs();
      const query = String(params.query ?? '').trim().toLowerCase();
      const sort = String(params.sort ?? 'cpu');
      const ascending = params.order === 'asc';
      const limit = Number(params.limit) || 250;
      let list = rows(t);
      const total = list.length;
      if (params.mine) list = list.filter((p) => p.own);
      if (query) {
        list = list.filter((p) => p.name.toLowerCase().includes(query) || (p.cmd ?? '').toLowerCase().includes(query) || String(p.pid) === query);
      }
      const matched = list.length;
      const key = sort as keyof ProcessRow;
      list.sort((a, b) => {
        const x = a[key];
        const y = b[key];
        const cmp = typeof x === 'number' && typeof y === 'number' ? x - y : String(x ?? '').localeCompare(String(y ?? ''));
        return ascending ? cmp : -cmp;
      });
      const states: Record<string, number> = {};
      for (const p of list) states[p.state] = (states[p.state] ?? 0) + 1;
      return { processes: list.slice(0, limit), total, matched, threads: list.reduce((a, p) => a + p.threads, 0), states, cores: THREADS, uid: 1000 };
    },

    process(params) {
      const pid = Number(params.pid);
      const p = PROCS.find((x) => x.pid === pid);
      if (!p) throw new Error('that process has ended');
      const t = secs();
      return {
        pid,
        name: p.name,
        cmd: p.cmd,
        exe: p.cmd.split(' ')[0] || null,
        cwd: p.uid === 1000 ? '/home/morvoso' : '/',
        state: 'S (sleeping)',
        ppid: p.ppid,
        threads: p.threads,
        vmPeak: p.rss * 4.2,
        vmSize: p.rss * 3.4,
        vmRss: p.rss,
        vmSwap: p.weight > 0.2 ? 12 * MIB : 0,
        fds: 12 + p.threads * 2,
        read: wave(t, pid % 7) * 400 * MIB,
        written: wave(t, pid % 11) * 120 * MIB,
        cgroup: p.uid === 1000 ? `/user.slice/user-1000.slice/session.scope/${p.name}` : `/system.slice/${p.name}.service`,
        wine: !!p.wine,
        voluntary: Math.round(wave(t, pid % 5) * 90000),
        involuntary: Math.round(wave(t, pid % 3) * 4000),
      } satisfies ProcessDetail;
    },

    kill(params) {
      const pid = Number(params.pid);
      ended.add(pid);
      console.info('[mock] signalled', pid, params.signal ?? 'TERM');
      return { pid };
    },

    containers(params) {
      const t = secs();
      const stats = !!params.stats;
      const build = (engine: 'docker' | 'podman'): ContainerRow[] =>
        CONTAINERS.filter((c) => c.engine === engine).map(({ load, ...base }) => {
          const row: ContainerRow = { ...base, cpu: null, mem: null, memPercent: null, net: null, block: null, pids: null };
          if (stats && base.running) {
            const use = load * (0.5 + wave(t, base.id.charCodeAt(0) % 13));
            row.cpu = use * 40;
            row.mem = `${(use * 800).toFixed(1)}MiB / 4GiB`;
            row.memPercent = use * 20;
            row.net = '412MB / 88MB';
            row.block = '1.2GB / 340MB';
            row.pids = String(4 + Math.round(use * 20));
          }
          return row;
        });
      // `running` is the engine answering, not a container count.
      return {
        docker: { available: true, running: true, containers: build('docker') },
        podman: { available: true, running: true, containers: build('podman') },
      };
    },

    services() {
      const scope = (units: UnitRow[]) => ({
        available: true,
        running: units.filter((u) => u.sub === 'running').length,
        failed: units.filter((u) => u.active === 'failed'),
        units,
      });
      return { available: true, system: scope(SYSTEM_UNITS), user: scope(USER_UNITS) };
    },
  };
}
