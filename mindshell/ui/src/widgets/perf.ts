import { every, h } from '../dom';
import { icon } from '../icons';
import { modeInfo, perfRefresh, perfSubscribe } from '../perf';
import { registerWidget } from './registry';
import { panelItem } from './common';

registerWidget({
  type: 'perf',
  name: 'Performance',
  description: 'The performance mode: balanced, performance or quiet. Click to switch.',
  icon: 'rocket',
  containers: ['panel'],
  defaults: { label: false },
  settings: { label: { label: 'Show the mode name', type: 'boolean' } },
  create(ctx) {
    const el = panelItem(ctx, 'w-perf', 'Performance mode');
    const ic = h('span', { class: 'w-ic' });
    const label = h('span', { class: 'w-label' });
    el.append(ic, label);
    let cfg = ctx.config;
    let current = '';
    let game = false;
    const render = () => {
      const m = modeInfo(current);
      ic.replaceChildren(icon(current ? m.icon : 'gauge', 18));
      label.textContent = current ? m.label : '…';
      label.hidden = !cfg.label || !!ctx.panel?.vertical;
      el.dataset.mode = current;
      el.classList.toggle('game', game);
      el.title = current ? `${m.label} mode${game ? ' · a game is running' : ''}` : 'Performance mode';
    };
    render();
    perfSubscribe(el, (s) => {
      current = s?.effective || s?.mode || '';
      game = !!s && s.game > 0;
      render();
    });
    void perfRefresh();
    every(el, 10000, () => void perfRefresh());
    el.addEventListener('click', () => ctx.togglePopup('perf', {}, { anchor: ctx.anchorOf(el) }));
    ctx.store.bind(el, 'popups', () => el.classList.toggle('open', ctx.store.popups.has('perf')));
    return {
      el,
      update(c) {
        cfg = c;
        render();
      },
    };
  },
});
