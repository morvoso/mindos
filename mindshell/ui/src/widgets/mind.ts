import * as bridge from '../bridge';
import { every, h } from '../dom';
import { icon } from '../icons';
import { registerWidget } from './registry';
import { panelItem } from './common';
import type { MindStatus } from '../types';

registerWidget({
  type: 'mind',
  name: 'Mind',
  description: 'Status of the Mind assistant. Click to open the Mind bar (Super+Space).',
  icon: 'mind',
  containers: ['panel'],
  defaults: { label: true, model: false },
  settings: {
    label: { label: 'Show the MIND label', type: 'boolean', help: 'Off: just the icon and the status dot' },
    model: { label: 'Show the model name', type: 'boolean' },
  },
  create(ctx) {
    const el = panelItem(ctx, 'w-mind', 'Mind · Super+Space');
    const dot = h('span', { class: 'mind-dot' });
    const label = h('span', { class: 'w-label' }, 'MIND');
    const model = h('span', { class: 'w-sub mono' });
    el.append(h('span', { class: 'w-ic' }, icon('mind', 18)), label, dot, model);
    let cfg = ctx.config;
    const render = () => {
      const m = ctx.store.state.mind;
      const st = !m?.connected ? 'off' : m.ready ? 'ready' : 'loading';
      el.dataset.state = st;
      model.textContent = m?.model ?? '';
      model.hidden = !cfg.model || !m?.model || !!ctx.panel?.vertical;
      label.hidden = !cfg.label || !!ctx.panel?.vertical;
      el.title = st === 'ready' ? `Mind ready${m?.model ? ' · ' + m.model : ''}` : st === 'loading' ? 'Mind is loading a model' : 'Mind is offline';
    };
    render();
    ctx.store.bind(el, 'mind', render);
    every(el, 15000, () =>
      bridge.call<MindStatus>('mind.status').then((m) => {
        ctx.store.state.mind = m;
        ctx.store.emit('mind');
      }).catch(() => undefined),
    );
    el.addEventListener('click', () => bridge.send('mind.toggle'));
    return {
      el,
      update(c) {
        cfg = c;
        render();
      },
    };
  },
});
