import { h } from '../dom';
import { icon } from '../icons';
import { registerWidget } from './registry';
import { panelItem } from './common';

registerWidget({
  type: 'notifications',
  name: 'Notifications',
  description: 'The bell: notifications from apps and what the Mind wants you to know. Click for the notification centre.',
  icon: 'bell',
  containers: ['panel'],
  defaults: { count: true },
  settings: { count: { label: 'Show the count', type: 'boolean', help: 'A badge with the number of unread notifications' } },
  create(ctx) {
    const el = panelItem(ctx, 'w-notifications', 'Notifications');
    const ic = h('span', { class: 'w-ic' });
    const badge = h('span', { class: 'w-badge' });
    el.append(ic, badge);
    let cfg = ctx.config;
    const render = () => {
      const st = ctx.store.state;
      const dnd = !!st.notify?.dnd;
      const n = ctx.store.attention();
      const danger = ctx.store.notices().some((x) => x.level === 'danger');
      ic.replaceChildren(icon(dnd ? 'bell-off' : 'bell', 18));
      badge.textContent = n > 99 ? '99+' : String(n);
      badge.hidden = !cfg.count || n === 0;
      el.classList.toggle('dnd', dnd);
      el.classList.toggle('has', n > 0);
      el.classList.toggle('danger', danger);
      el.title = dnd ? 'Do not disturb is on' : n ? `${n} waiting` : 'No notifications';
    };
    render();
    ctx.store.bind(el, 'notify', render);
    ctx.store.bind(el, 'mindNotices', render);
    ctx.store.bind(el, 'mind', render);
    el.addEventListener('click', () => ctx.togglePopup('notifications', {}, { anchor: ctx.anchorOf(el) }));
    ctx.store.bind(el, 'popups', () => el.classList.toggle('open', ctx.store.popups.has('notifications')));
    return {
      el,
      update(c) {
        cfg = c;
        render();
      },
    };
  },
});
