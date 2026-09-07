import { h } from '../dom';
import { icon } from '../icons';
import { registerWidget } from './registry';
import { panelItem } from './common';

registerWidget({
  type: 'power',
  name: 'Power',
  description: 'Shut down, restart, suspend or log out.',
  icon: 'power',
  containers: ['panel'],
  defaults: { label: false },
  settings: { label: { label: 'Show the label', type: 'boolean' } },
  create(ctx) {
    const el = panelItem(ctx, 'w-power', 'Power');
    const label = h('span', { class: 'w-label' }, 'Power');
    el.append(h('span', { class: 'w-ic' }, icon('power', 18)), label);
    let cfg = ctx.config;
    const render = () => {
      label.hidden = !cfg.label || !!ctx.panel?.vertical;
    };
    render();
    el.addEventListener('click', () => ctx.togglePopup('power', {}, { anchor: ctx.anchorOf(el) }));
    ctx.store.bind(el, 'popups', () => el.classList.toggle('open', ctx.store.popups.has('power')));
    return {
      el,
      update(c) {
        cfg = c;
        render();
      },
    };
  },
});
