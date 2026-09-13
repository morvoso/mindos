// The system readout: the machine at a glance, in a card narrow enough for
// the desktop rail, with a way through to the Task Manager for the rest.
//
// It is deliberately the cheapest complete picture there is. One call every
// three seconds gets the processor, the memory, the graphics, the network,
// the disks and the three busiest processes; the container engines are asked
// about once every ten of those beats, because a container that appeared two
// seconds ago can wait and a subprocess every three seconds cannot. Nothing is
// sampled while the rail is scrolled away, hidden behind an open page, or
// while a game is running — the desktop stops entirely then (see quiet.ts).

import { h } from './dom';
import { icon } from './icons';
import { bytes, chart, count, heat, meter, percent as pct, poll, rate } from './monitor';
import type { Demand } from './monitor';
import { openTaskManager } from './tasks-open';
import type { Overview } from './types';

export interface ReadoutOptions {
  /** How often to sample, in milliseconds. */
  intervalMs?: number;
  /** Show the three busiest processes. */
  processes?: boolean;
  /** Show the scrolling processor and graphics graph. */
  graph?: boolean;
  /** Show the network, disk and container line. */
  traffic?: boolean;
  /** Show the link through to the Task Manager. */
  link?: boolean;
  /** How many readings the graph holds. */
  points?: number;
  /** Called with every sample, for a caller that shows a line of its own. */
  onSample?(v: Overview): void;
}

/** How many beats apart the container engines are asked anything. */
const CONTAINER_EVERY = 10;

export function systemReadout(options: ReadoutOptions = {}): { el: HTMLElement; destroy(): void } {
  const { processes = true, graph = true, traffic = true, link = true } = options;
  const el = h('div', { class: 'readout' });

  const load = chart({ points: options.points ?? 48, colors: ['var(--accent)', 'var(--mind)'], max: 100, height: 44, class: 'readout-chart' });
  const cpu = meter('CPU');
  const gpu = meter('GPU');
  const mem = meter('Memory');
  const down = h('span', { class: 'readout-flow', title: 'Network in' }, icon('download', 11), h('b', {}, '—'));
  const up = h('span', { class: 'readout-flow', title: 'Network out' }, icon('upload', 11), h('b', {}, '—'));
  const disk = h('span', { class: 'readout-flow', title: 'Disk traffic' }, icon('hdd', 11), h('b', {}, '—'));
  const boxes = h('span', { class: 'readout-flow', hidden: true, title: 'Containers' }, icon('docker', 11), h('b', {}, ''));
  const flows = h('div', { class: 'readout-flows' }, down, up, disk, boxes);
  const top = h('ul', { class: 'readout-top' });
  const empty = h('p', { class: 'readout-empty gaming-meta' }, 'Reading the machine…');

  if (graph) el.append(load.el);
  el.append(cpu.el, gpu.el, mem.el);
  if (traffic) el.append(flows);
  if (processes) el.append(top);
  el.append(empty);
  if (link) {
    el.append(
      h(
        'button',
        { class: 'gaming-text-action readout-open', onclick: () => openTaskManager() },
        'Open Task Manager',
        icon('arrow-right', 12),
      ),
    );
  }

  // The demand is a live object: the shared sampler reads it at request time,
  // so adding a part here is how this readout asks for more on one beat and
  // goes back to the cheap question on the next.
  const demand: Demand = { parts: ['net', 'storage', 'containers'], top: processes ? 3 : 0 };
  let beat = 0;

  const paint = (v: Overview) => {
    empty.hidden = true;
    cpu.set(v.cpu.usage / 100, pct(v.cpu.usage), v.cpu.temp != null ? `${Math.round(v.cpu.temp)}° · ${v.cpu.threads} threads` : `${v.cpu.threads} threads`);
    const card = v.gpus[0];
    if (card?.util != null) {
      const vram = card.mem != null && card.memTotal ? `${bytes(card.mem * 1048576, 0)} / ${bytes(card.memTotal * 1048576, 0)}` : '';
      gpu.set(card.util / 100, pct(card.util), [card.temp != null ? `${Math.round(card.temp)}°` : '', vram].filter(Boolean).join(' · '));
      gpu.el.hidden = false;
    } else {
      gpu.el.hidden = v.gpus.length === 0;
      if (card) gpu.set(0, '—', card.name);
    }
    const usedFraction = v.memory.total ? v.memory.used / v.memory.total : 0;
    mem.set(
      usedFraction,
      `${bytes(v.memory.used)} / ${bytes(v.memory.total, 0)}`,
      v.memory.swapUsed > 0 ? `${bytes(v.memory.swapUsed)} swapped` : '',
    );
    if (graph) load.push(v.cpu.usage, card?.util ?? 0);

    if (traffic) {
      let rx = 0;
      let tx = 0;
      for (const n of v.net ?? []) {
        if (n.kind === 'bridge' || n.kind === 'virtual') continue;
        rx += n.rxRate;
        tx += n.txRate;
      }
      let io = 0;
      for (const d of v.disks ?? []) io += d.readRate + d.writeRate;
      (down.lastElementChild as HTMLElement).textContent = rate(rx);
      (up.lastElementChild as HTMLElement).textContent = rate(tx);
      (disk.lastElementChild as HTMLElement).textContent = rate(io);
      const engines = v.containers;
      if (engines) {
        const running = engines.docker.running + engines.podman.running;
        const known = engines.docker.available || engines.podman.available;
        boxes.hidden = !known;
        (boxes.lastElementChild as HTMLElement).textContent = running ? `${running} up` : 'idle';
      }
    }

    if (processes) {
      const rows = v.top ?? [];
      top.replaceChildren(
        ...rows.map((p) =>
          h(
            'li',
            { title: `${p.name} — pid ${p.pid}` },
            h('span', { class: 'readout-proc-name' }, p.name),
            h('span', { class: 'readout-proc-cpu', style: { color: heat(p.cpu) } }, `${p.cpu.toFixed(1)}%`),
          ),
        ),
      );
      if (rows.length === 0) top.replaceChildren(h('li', { class: 'gaming-meta' }, `${count(v.cpu.procs)} processes`));
    }
  };

  const stop = poll(
    el,
    options.intervalMs ?? 3000,
    demand,
    (v) => {
      beat += 1;
      options.onSample?.(v);
      demand.parts = beat % CONTAINER_EVERY === 0 ? ['net', 'storage', 'containers'] : ['net', 'storage'];
      paint(v);
    },
    () => {
      empty.hidden = false;
      empty.textContent = 'System readings unavailable';
    },
  );
  return { el, destroy: stop };
}
