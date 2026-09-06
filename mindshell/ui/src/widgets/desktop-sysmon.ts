import * as bridge from '../bridge';
import { every, h } from '../dom';
import { registerWidget } from './registry';
import { pct } from './common';
import type { Stats } from '../types';

const N = 90;
const SERIES = [
  { key: 'cpu', name: 'CPU', color: '#19e3ff' },
  { key: 'gpu', name: 'GPU', color: '#a78bfa' },
  { key: 'mem', name: 'MEM', color: '#8b9bb0' },
] as const;

registerWidget({
  type: 'desktop-sysmon',
  name: 'Load graph',
  description: 'Rolling CPU, GPU and memory graph for the desktop.',
  icon: 'gpu',
  containers: ['desktop'],
  defaults: { interval: 2, cpu: true, gpu: true, memory: true },
  defaultSize: { w: 360, h: 180 },
  settings: {
    interval: { label: 'Refresh interval (s)', type: 'number', min: 1, max: 30, step: 1 },
    cpu: { label: 'CPU', type: 'boolean' },
    gpu: { label: 'GPU', type: 'boolean' },
    memory: { label: 'Memory', type: 'boolean' },
  },
  create(ctx) {
    const canvas = h('canvas', { class: 'dsys-canvas' }) as HTMLCanvasElement;
    const legend = h('div', { class: 'dsys-legend' });
    const legendVals = new Map<string, HTMLElement>();
    for (const s of SERIES) {
      const v = h('span', { class: 'mono' }, '--');
      legendVals.set(s.key, v);
      legend.appendChild(h('span', { class: 'dsys-key', dataset: { key: s.key } }, h('i', { style: { background: s.color } }), s.name, v));
    }
    const el = h('div', { class: 'dw-body dsys' }, h('div', { class: 'dw-title' }, 'SYSTEM LOAD'), canvas, legend);
    let cfg = ctx.config;
    const hist: Record<string, number[]> = { cpu: [], gpu: [], mem: [] };
    let stats: Stats | undefined;
    const enabled = (k: string) => (k === 'cpu' ? !!cfg.cpu : k === 'gpu' ? !!cfg.gpu && !!stats?.gpu : !!cfg.memory);

    const draw = () => {
      const w = canvas.clientWidth;
      const hgt = canvas.clientHeight;
      if (!w || !hgt) return;
      const dpr = Math.min(2, window.devicePixelRatio || 1);
      if (canvas.width !== Math.round(w * dpr) || canvas.height !== Math.round(hgt * dpr)) {
        canvas.width = Math.round(w * dpr);
        canvas.height = Math.round(hgt * dpr);
      }
      const g = canvas.getContext('2d');
      if (!g) return;
      g.setTransform(dpr, 0, 0, dpr, 0, 0);
      g.clearRect(0, 0, w, hgt);
      g.strokeStyle = 'rgba(34,48,65,.9)';
      g.lineWidth = 1;
      for (let i = 1; i < 4; i++) {
        const y = Math.round((hgt * i) / 4) + 0.5;
        g.beginPath();
        g.moveTo(0, y);
        g.lineTo(w, y);
        g.stroke();
      }
      for (const s of SERIES) {
        if (!enabled(s.key)) continue;
        const data = hist[s.key];
        if (data.length < 2) continue;
        g.beginPath();
        const step = w / (N - 1);
        const start = N - data.length;
        data.forEach((v, i) => {
          const x = (start + i) * step;
          const y = hgt - 2 - (Math.min(100, v) / 100) * (hgt - 4);
          if (i === 0) g.moveTo(x, y);
          else g.lineTo(x, y);
        });
        g.strokeStyle = s.color;
        g.lineWidth = 1.5;
        g.lineJoin = 'round';
        g.stroke();
        g.lineTo(w, hgt);
        g.lineTo((start) * step, hgt);
        g.closePath();
        g.fillStyle = s.color + '14';
        g.fill();
      }
    };

    const push = (k: string, v: number | undefined) => {
      if (v === undefined) return;
      const a = hist[k];
      a.push(v);
      if (a.length > N) a.shift();
    };
    const render = () => {
      for (const s of SERIES) {
        const row = legend.querySelector<HTMLElement>(`[data-key="${s.key}"]`)!;
        row.hidden = !enabled(s.key);
        const v = s.key === 'cpu' ? stats?.cpu : s.key === 'gpu' ? stats?.gpu?.util : stats ? (stats.memUsed / stats.memTotal) * 100 : undefined;
        legendVals.get(s.key)!.textContent = v === undefined ? '--' : pct(v);
      }
      draw();
    };
    const poll = () =>
      bridge.call<Stats>('system.stats').then((s) => {
        stats = s;
        push('cpu', s.cpu);
        push('gpu', s.gpu?.util);
        push('mem', (s.memUsed / s.memTotal) * 100);
        render();
      }).catch(() => undefined);
    let stop = every(el, (Number(cfg.interval) || 2) * 1000, poll);
    const ro = new ResizeObserver(() => draw());
    ro.observe(canvas);
    return {
      el,
      update(c) {
        cfg = c;
        stop();
        stop = every(el, (Number(cfg.interval) || 2) * 1000, poll);
        render();
      },
      destroy() {
        stop();
        ro.disconnect();
      },
    };
  },
});
