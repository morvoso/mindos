// The two pages that watch the machine as a whole: Overview, which is the
// dashboard, and Performance, which is the same hardware in detail.

import { h } from '../dom';
import { icon } from '../icons';
import { bytes, count, degrees, duration, heat, hertz, percent, poll, rate } from '../monitor';
import type { GpuInfo, Overview, ProcessRow } from '../types';
import { barRow, composition, dataTable, factGrid, hero, panel } from './tasks-parts';
import { pageHeader } from './shared';

/** How often the app samples. Slower than a second would feel dead; faster
 *  would be measuring the measurement. */
const TICK = 1000;

export function overviewPage(el: HTMLElement): () => void {
  const head = pageHeader('Overview', 'Reading the machine…');
  const sub = head.querySelector<HTMLElement>('.page-sub')!;

  const cpu = hero('Processor', 'cpu', { accent: 'var(--accent)' });
  const memory = hero('Memory', 'memory', { accent: 'var(--mind)' });
  const gpu = hero('Graphics', 'gpu', { accent: 'var(--ok)' });
  const network = hero('Network', 'ethernet', { accent: 'var(--warn)', max: 'auto', colors: ['var(--warn)', 'var(--accent)'], fill: true });
  const heroes = h('div', { class: 'tk-heroes' }, cpu.el, memory.el, gpu.el, network.el);

  const top = dataTable<ProcessRow>(
    [
      { key: 'name', label: 'Process', width: 'minmax(0,1fr)', cell: (p) => p.name },
      { key: 'pid', label: 'PID', width: '68px', align: 'end', cell: (p) => String(p.pid) },
      { key: 'cpu', label: 'CPU', width: '68px', align: 'end', cell: (p) => `${p.cpu.toFixed(1)}%`, tone: (p) => heat(p.cpu) },
      { key: 'rss', label: 'Memory', width: '84px', align: 'end', cell: (p) => bytes(p.rss) },
    ],
    (p) => String(p.pid),
    { empty: 'No processes to show.' },
  );

  const storage = h('div', { class: 'tk-barrows' });
  const bars = new Map<string, ReturnType<typeof barRow>>();
  const facts = factGrid([], 'tk-facts-wide');

  el.append(
    head,
    heroes,
    h(
      'div',
      { class: 'tk-split' },
      panel('Busiest processes', 'By processor share', top.el),
      h('div', { class: 'tk-column' }, panel('At a glance', undefined, facts.el), panel('Storage', 'Space in use', storage)),
    ),
  );

  const paint = (v: Overview) => {
    const host = v.host;
    sub.textContent = host ? `${host.hostname} · ${host.os} · up ${duration(host.uptime)}` : 'Live readings';

    cpu.chart.push(v.cpu.usage);
    cpu.set(percent(v.cpu.usage), v.cpu.usage / 100);
    cpu.setSub(v.cpu.model);
    cpu.setFacts([
      ['Clock', hertz(v.cpu.freqAvg)],
      ['Temperature', degrees(v.cpu.temp)],
      ['Cores', `${v.cpu.cores}C / ${v.cpu.threads}T`],
      ['Load', v.cpu.load.map((n) => n.toFixed(2)).join('  ')],
    ]);

    const usedFraction = v.memory.total ? v.memory.used / v.memory.total : 0;
    memory.chart.push(usedFraction * 100);
    memory.set(`${bytes(v.memory.used)}`, usedFraction);
    memory.setSub(`of ${bytes(v.memory.total, 0)} · ${percent(usedFraction * 100)} in use`);
    memory.setFacts([
      ['Available', bytes(v.memory.available)],
      ['Cached', bytes(v.memory.cached)],
      ['Swap', v.memory.swapTotal ? `${bytes(v.memory.swapUsed)} / ${bytes(v.memory.swapTotal, 0)}` : 'None'],
      ['Processes', `${count(v.cpu.procs)}`],
    ]);

    const card: GpuInfo | undefined = v.gpus[0];
    if (card) {
      gpu.el.hidden = false;
      gpu.chart.push(card.util ?? 0);
      gpu.set(card.util != null ? percent(card.util) : '—', (card.util ?? 0) / 100);
      gpu.setSub(card.name);
      gpu.setFacts([
        ['VRAM', card.mem != null && card.memTotal ? `${bytes(card.mem * 1048576)} / ${bytes(card.memTotal * 1048576, 0)}` : '—'],
        ['Temperature', degrees(card.temp)],
        ['Power', card.power != null ? `${Math.round(card.power)} W` : '—'],
        ['Clock', hertz(card.clock)],
      ]);
    } else {
      gpu.el.hidden = true;
    }

    let rx = 0;
    let tx = 0;
    let live = 0;
    for (const n of v.net ?? []) {
      if (n.kind === 'bridge' || n.kind === 'virtual') continue;
      rx += n.rxRate;
      tx += n.txRate;
      if (n.state === 'up') live += 1;
    }
    network.chart.push(rx, tx);
    // The headline is everything crossing the wire; the facts below split it.
    network.set(rate(rx + tx));
    network.setSub(`${live} interface${live === 1 ? '' : 's'} up`);
    let read = 0;
    let write = 0;
    for (const d of v.disks ?? []) {
      read += d.readRate;
      write += d.writeRate;
    }
    network.setFacts([
      ['Download', rate(rx)],
      ['Upload', rate(tx)],
      ['Disk read', rate(read)],
      ['Disk write', rate(write)],
    ]);

    top.set(v.top ?? []);

    const hottest = (v.sensors?.temps ?? []).reduce<{ chip: string; label: string; value: number } | undefined>(
      (best, t) => (!best || t.value > best.value ? t : best),
      undefined,
    );
    const engines = v.containers;
    const containers = engines ? engines.docker.running + engines.podman.running : 0;
    const known = engines ? engines.docker.available || engines.podman.available : false;
    facts.set([
      ['Threads running', `${v.cpu.running} of ${count(v.cpu.procs)}`],
      ['Context switches', `${count(v.cpu.ctxtRate)}/s`],
      ['Interrupts', `${count(v.cpu.intrRate)}/s`],
      ['New processes', `${v.cpu.forkRate.toFixed(1)}/s`],
      ['Hottest sensor', hottest ? `${degrees(hottest.value)} ${hottest.chip}` : '—'],
      ['Containers', known ? `${containers} running` : 'None installed'],
      ['Kernel', v.host?.kernel ?? '—'],
      ['Uptime', v.host ? duration(v.host.uptime) : '—'],
    ]);

    const filesystems = (v.filesystems ?? []).slice(0, 5);
    for (const fs of filesystems) {
      let bar = bars.get(fs.mount);
      if (!bar) {
        bar = barRow();
        bars.set(fs.mount, bar);
        storage.appendChild(bar.el);
      }
      bar.set(fs.mount, fs.fstype, fs.percent / 100, `${bytes(fs.avail)} free`);
    }
    for (const [mount, bar] of bars) {
      if (!filesystems.some((f) => f.mount === mount)) {
        bar.el.remove();
        bars.delete(mount);
      }
    }
  };

  // Overview draws every part of the machine, so it asks for all of them.
  return poll(el, TICK, { top: 8 }, paint, () => (sub.textContent = 'System readings unavailable'));
}

export function performancePage(el: HTMLElement): () => void {
  const head = pageHeader('Performance', 'Processor, memory and graphics in detail');

  // ---- processor
  const cpuChart = hero('Processor', 'cpu', { accent: 'var(--accent)', height: 128, points: 120 });
  const cores = h('div', { class: 'tk-cores' });
  const coreBars: HTMLElement[] = [];
  const cpuFacts = factGrid([], 'tk-facts-wide');
  const kinds = composition();

  // ---- memory
  const memChart = hero('Memory', 'memory', { accent: 'var(--mind)', height: 128, points: 120 });
  const memComp = composition();
  const swap = h('div', { class: 'tk-barrows' });
  const swapBars = new Map<string, ReturnType<typeof barRow>>();
  const memFacts = factGrid([], 'tk-facts-wide');

  // ---- graphics
  const gpuHost = h('div', { class: 'tk-column' });
  const cards = new Map<string, { hero: ReturnType<typeof hero>; vram: ReturnType<typeof barRow>; facts: ReturnType<typeof factGrid>; el: HTMLElement }>();

  el.append(
    head,
    panel('Processor', 'Every hardware thread', cpuChart.el, h('div', { class: 'tk-subhead' }, 'Per-thread load'), cores, h('div', { class: 'tk-subhead' }, 'Where the time goes'), kinds.el, cpuFacts.el),
    panel('Memory', 'What is holding it', memChart.el, memComp.el, h('div', { class: 'tk-subhead' }, 'Swap'), swap, memFacts.el),
    panel('Graphics', 'Every card the machine will talk about', gpuHost),
  );

  const paint = (v: Overview) => {
    cpuChart.chart.push(v.cpu.usage);
    cpuChart.set(percent(v.cpu.usage), v.cpu.usage / 100);
    cpuChart.setSub(`${v.cpu.model} · ${v.cpu.cores} cores, ${v.cpu.threads} threads`);
    cpuChart.setFacts([
      ['Average clock', hertz(v.cpu.freqAvg)],
      ['Maximum', hertz(v.cpu.freqMax)],
      ['Temperature', v.cpu.temp != null ? `${degrees(v.cpu.temp)} ${v.cpu.tempLabel ?? ''}`.trim() : '—'],
      ['Governor', v.cpu.governor ?? '—'],
    ]);

    if (coreBars.length !== v.cpu.perCore.length) {
      coreBars.length = 0;
      cores.replaceChildren(
        ...v.cpu.perCore.map((_, i) => {
          const fill = h('i');
          const node = h('div', { class: 'tk-core' }, h('span', { class: 'tk-core-fill' }, fill), h('small', {}, String(i)));
          coreBars.push(fill);
          return node;
        }),
      );
    }
    v.cpu.perCore.forEach((value, i) => {
      const fill = coreBars[i];
      fill.style.height = `${Math.max(2, value).toFixed(1)}%`;
      fill.style.background = heat(value);
      (fill.parentElement!.parentElement as HTMLElement).title = `Thread ${i} — ${percent(value)}${v.cpu.freq[i] ? ` at ${hertz(v.cpu.freq[i])}` : ''}`;
    });

    const k = v.cpu.kinds;
    const idle = Math.max(0, 100 - k.user - k.system - k.iowait - k.irq);
    kinds.set(
      [
        { label: 'Programs', value: k.user, color: 'var(--accent)', hint: percent(k.user, 1) },
        { label: 'Kernel', value: k.system, color: 'var(--mind)', hint: percent(k.system, 1) },
        { label: 'Waiting on disk', value: k.iowait, color: 'var(--warn)', hint: percent(k.iowait, 1) },
        { label: 'Interrupts', value: k.irq, color: 'var(--danger)', hint: percent(k.irq, 1) },
        { label: 'Idle', value: idle, color: 'var(--surface-strong)', hint: percent(idle, 1) },
      ],
      100,
    );
    cpuFacts.set([
      ['Load average', v.cpu.load.map((n) => n.toFixed(2)).join('   ')],
      ['Scaling driver', v.cpu.driver ?? '—'],
      ['Power preference', v.cpu.epp ?? '—'],
      ['Context switches', `${count(v.cpu.ctxtRate)}/s`],
      ['Interrupts', `${count(v.cpu.intrRate)}/s`],
      ['New processes', `${v.cpu.forkRate.toFixed(1)}/s`],
      ['Processes', count(v.cpu.procs)],
      ['Running now', String(v.cpu.running)],
    ]);

    const m = v.memory;
    const usedFraction = m.total ? m.used / m.total : 0;
    memChart.chart.push(usedFraction * 100);
    memChart.set(bytes(m.used), usedFraction);
    memChart.setSub(`${percent(usedFraction * 100)} of ${bytes(m.total, 0)} in use`);
    memChart.setFacts([
      ['Available', bytes(m.available)],
      ['Cached', bytes(m.cached)],
      ['Shared', bytes(m.shared)],
      ['Kernel', bytes(m.slab + m.kernel)],
    ]);
    memComp.set(
      [
        { label: 'In use', value: m.used, color: 'var(--mind)', hint: bytes(m.used) },
        { label: 'Cached', value: Math.max(0, m.cached), color: 'var(--accent-dim)', hint: bytes(Math.max(0, m.cached)) },
        { label: 'Buffers', value: m.buffers, color: 'var(--ok)', hint: bytes(m.buffers) },
        { label: 'Free', value: m.free, color: 'var(--surface-strong)', hint: bytes(m.free) },
      ],
      m.total,
    );
    const swapKeys: string[] = [];
    for (const z of m.zram) {
      swapKeys.push(z.name);
      let bar = swapBars.get(z.name);
      if (!bar) {
        bar = barRow();
        swapBars.set(z.name, bar);
        swap.appendChild(bar.el);
      }
      const ratio = z.compressed > 0 ? z.stored / z.compressed : 0;
      bar.set(
        z.name,
        `${z.algorithm ?? 'compressed'}${ratio > 1 ? ` · ${ratio.toFixed(1)}× smaller` : ''}`,
        z.size ? z.used / z.size : 0,
        `${bytes(z.used)} of ${bytes(z.size, 0)}`,
      );
    }
    for (const s of m.swaps) {
      if (s.name.includes('zram')) continue;
      swapKeys.push(s.name);
      let bar = swapBars.get(s.name);
      if (!bar) {
        bar = barRow();
        swapBars.set(s.name, bar);
        swap.appendChild(bar.el);
      }
      bar.set(s.name, s.kind, s.size ? s.used / s.size : 0, `${bytes(s.used)} of ${bytes(s.size, 0)}`);
    }
    for (const [name, bar] of swapBars) {
      if (!swapKeys.includes(name)) {
        bar.el.remove();
        swapBars.delete(name);
      }
    }
    if (swapKeys.length === 0 && !swap.firstElementChild) swap.append(h('p', { class: 'tk-empty' }, 'No swap is configured.'));
    memFacts.set([
      ['Dirty pages', bytes(m.dirty)],
      ['Slab', bytes(m.slab)],
      ['Page tables', bytes(m.kernel)],
      ['Swap free', m.swapTotal ? bytes(m.swapFree) : '—'],
    ]);

    const seen: string[] = [];
    v.gpus.forEach((card, i) => {
      const key = `${card.vendor}:${card.name}:${i}`;
      seen.push(key);
      let entry = cards.get(key);
      if (!entry) {
        const graph = hero(card.name, 'gpu', { accent: 'var(--ok)', height: 112, points: 120 });
        const vram = barRow();
        const facts = factGrid([], 'tk-facts-wide');
        const wrap = h('div', { class: 'tk-gpu' }, graph.el, vram.el, facts.el);
        entry = { hero: graph, vram, facts, el: wrap };
        cards.set(key, entry);
        gpuHost.appendChild(wrap);
      }
      entry.hero.chart.push(card.util ?? 0);
      entry.hero.set(card.util != null ? percent(card.util) : '—', (card.util ?? 0) / 100);
      entry.hero.setSub(`${card.vendor}${card.driver ? ` · driver ${card.driver}` : ''}`);
      entry.hero.setFacts([
        ['Temperature', degrees(card.temp)],
        ['Power', card.power != null ? `${Math.round(card.power)} W${card.powerLimit ? ` of ${Math.round(card.powerLimit)} W` : ''}` : '—'],
        ['Core clock', hertz(card.clock)],
        ['Memory clock', hertz(card.memClock)],
      ]);
      if (card.memTotal) {
        entry.vram.el.hidden = false;
        entry.vram.set('Video memory', card.memUtil != null ? `bus ${percent(card.memUtil)}` : '', (card.mem ?? 0) / card.memTotal, `${bytes((card.mem ?? 0) * 1048576)} of ${bytes(card.memTotal * 1048576, 0)}`);
      } else {
        entry.vram.el.hidden = true;
      }
      entry.facts.set([
        ['Fan', card.fanPercent != null ? percent(card.fanPercent) : card.fan != null ? `${Math.round(card.fan)} rpm` : '—'],
        ['Vendor', card.vendor],
      ]);
    });
    for (const [key, entry] of cards) {
      if (!seen.includes(key)) {
        entry.el.remove();
        cards.delete(key);
      }
    }
    if (v.gpus.length === 0 && !gpuHost.firstElementChild) {
      gpuHost.append(h('p', { class: 'tk-empty' }, h('span', {}, icon('gpu', 15)), ' No graphics card reports its load on this machine.'));
    }
  };

  // The processor, the memory and the cards come with every answer; this page
  // wants nothing beyond them, and an empty list says so.
  return poll(el, TICK, { parts: [] }, paint);
}
