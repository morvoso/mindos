import * as bridge from '../bridge';
import { every, h } from '../dom';
import { icon } from '../icons';
import { registerWidget } from './registry';
import { panelItem } from './common';
import type { MindStatus } from '../types';

/** What the icon's colour means, in the order the states are checked. */
function stateOf(m: MindStatus | undefined): { id: string; title: string } {
  if (!m || !m.connected) return { id: 'off', title: 'Mind is offline' };
  if (m.sleeping) return { id: 'asleep', title: 'Mind is asleep — it wakes on the next question' };
  if (!m.ready) return { id: 'loading', title: 'Mind is loading its model' };
  return { id: 'ready', title: 'Mind is ready' };
}

registerWidget({
  type: 'mind',
  name: 'Mind',
  description: 'The Mind assistant. The icon colour is its state; click to open the Mind bar (Super+Space).',
  icon: 'mind',
  containers: ['panel'],
  defaults: { label: false, model: false },
  settings: {
    label: { label: 'Show the name next to the icon', type: 'boolean' },
    model: { label: 'Show the model name', type: 'boolean' },
  },
  create(ctx) {
    const el = panelItem(ctx, 'w-mind', 'Mind · Super+Space');
    const label = h('span', { class: 'w-label' }, 'Mind');
    const model = h('span', { class: 'w-sub mono' });
    el.append(h('span', { class: 'w-ic' }, icon('mind', 20)), label, model);
    let cfg = ctx.config;
    const render = () => {
      const m = ctx.store.state.mind;
      const st = stateOf(m);
      el.dataset.state = st.id;
      model.textContent = m?.model ?? '';
      model.hidden = !cfg.model || !m?.model || !!ctx.panel?.vertical;
      label.hidden = !cfg.label || !!ctx.panel?.vertical;
      el.title = `${st.title}${m?.model ? ` · ${m.model}` : ''}`;
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
