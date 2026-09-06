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
  defaults: {},
  create(ctx) {
    const el = panelItem(ctx, 'w-power', 'Power');
    el.append(h('span', { class: 'w-ic' }, icon('power', 18)));
    el.addEventListener('click', () => ctx.togglePopup('power', {}, { anchor: ctx.anchorOf(el) }));
    ctx.store.bind(el, 'popups', () => el.classList.toggle('open', ctx.store.popups.has('power')));
    return { el };
  },
});
