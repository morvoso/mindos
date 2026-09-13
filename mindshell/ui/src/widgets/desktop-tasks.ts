// The Task Manager's readout, as a desktop widget: the same component the
// gaming and productivity rails carry, so all three share one sampler and one
// walk of `/proc` per beat no matter how many of them are on screen.

import { h } from '../dom';
import { systemReadout } from '../readout';
import { registerWidget } from './registry';

registerWidget({
  type: 'desktop-tasks',
  name: 'System readout',
  description: 'Load, memory, traffic and the busiest processes, with a way into the Task Manager.',
  icon: 'gauge',
  containers: ['desktop'],
  defaults: { title: 'SYSTEM', interval: 3, graph: true, processes: true, traffic: true, link: true },
  defaultSize: { w: 300, h: 340 },
  settings: {
    title: { label: 'Title', type: 'string', placeholder: 'SYSTEM' },
    graph: { label: 'Load graph', type: 'boolean' },
    traffic: { label: 'Network and disk', type: 'boolean' },
    processes: { label: 'Busiest processes', type: 'boolean' },
    link: { label: 'Task Manager link', type: 'boolean' },
    interval: { label: 'Refresh every', type: 'number', min: 1, max: 30, step: 1, unit: 's' },
  },
  create(ctx) {
    const title = h('div', { class: 'dw-title' });
    const body = h('div', { class: 'dtasks-body' });
    const el = h('div', { class: 'dw-body dtasks' }, title, body);
    let cfg = ctx.config;
    let readout: { el: HTMLElement; destroy(): void } | undefined;

    // Every option changes what the readout asks the host for, so a settings
    // change builds a new one rather than hiding parts of the old.
    const build = () => {
      readout?.destroy();
      readout = systemReadout({
        intervalMs: Math.max(1, Number(cfg.interval) || 3) * 1000,
        graph: cfg.graph !== false,
        traffic: cfg.traffic !== false,
        processes: cfg.processes !== false,
        link: cfg.link !== false,
      });
      body.replaceChildren(readout.el);
    };
    const render = () => {
      title.textContent = String(cfg.title ?? 'SYSTEM');
      title.hidden = !title.textContent;
    };
    render();
    build();
    return {
      el,
      update(c) {
        cfg = c;
        render();
        build();
      },
      destroy() {
        readout?.destroy();
      },
    };
  },
});
