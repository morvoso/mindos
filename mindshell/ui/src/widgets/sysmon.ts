import * as bridge from '../bridge';
import { every, h } from '../dom';
import { registerWidget } from './registry';
import { pct } from './common';
import type { Stats } from '../types';

interface Meter {
  el: HTMLElement;
  bar: HTMLElement;
  val: HTMLElement;
}

function meter(name: string): Meter {
  const bar = h('i');
  const val = h('span', { class: 'meter-val mono' }, '--');
  const el = h('div', { class: `meter meter-${name.toLowerCase()}` }, h('span', { class: 'meter-name' }, name), val, h('span', { class: 'meter-bar' }, bar));
  return { el, bar, val };
}

function setMeter(m: Meter, v: number | undefined) {
  const p = v === undefined ? 0 : Math.max(0, Math.min(100, v));
  m.bar.style.transform = `scaleX(${p / 100})`;
  m.val.textContent = v === undefined ? '--' : pct(p);
  m.el.classList.toggle('hot', p >= 90);
}

registerWidget({
  type: 'sysmon',
  name: 'System monitor',
  description: 'CPU, memory and GPU load at a glance.',
  icon: 'cpu',
  containers: ['panel'],
  defaults: { cpu: true, memory: true, gpu: true, interval: 2 },
  settings: {
    cpu: { label: 'CPU', type: 'boolean' },
    memory: { label: 'Memory', type: 'boolean' },
    gpu: { label: 'GPU', type: 'boolean' },
    interval: { label: 'Refresh every', type: 'number', min: 1, max: 30, step: 1, unit: 's' },
  },
  create(ctx) {
    const cpu = meter('CPU');
    const mem = meter('MEM');
    const gpu = meter('GPU');
    const el = h('div', { class: 'w w-sysmon', title: 'System load' }, cpu.el, mem.el, gpu.el);
    let cfg = ctx.config;
    let stats: Stats | undefined;
    const render = () => {
      cpu.el.hidden = !cfg.cpu;
      mem.el.hidden = !cfg.memory;
      gpu.el.hidden = !cfg.gpu || (!!stats && !stats.gpu);
      setMeter(cpu, stats?.cpu);
      setMeter(mem, stats ? (stats.memUsed / stats.memTotal) * 100 : undefined);
      setMeter(gpu, stats?.gpu?.util);
      if (stats) {
        const parts = [`CPU ${pct(stats.cpu)}`, `MEM ${(stats.memUsed / 2 ** 30).toFixed(1)} / ${(stats.memTotal / 2 ** 30).toFixed(0)} GiB`];
        if (stats.gpu) parts.push(`GPU ${pct(stats.gpu.util)}${stats.gpu.temp !== undefined ? ` ${Math.round(stats.gpu.temp)}°C` : ''}`);
        el.title = parts.join(' · ');
      }
    };
    const poll = () =>
      bridge.call<Stats>('system.stats').then((s) => {
        stats = s;
        render();
      }).catch(() => undefined);
    render();
    poll();
    let stop = every(el, (Number(cfg.interval) || 2) * 1000, poll);
    return {
      el,
      update(c) {
        cfg = c;
        stop();
        stop = every(el, (Number(cfg.interval) || 2) * 1000, poll);
        render();
      },
      destroy: () => stop(),
    };
  },
});
