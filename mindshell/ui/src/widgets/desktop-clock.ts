import { h, pad2 } from '../dom';
import { registerWidget } from './registry';
import { formatDate, formatTime, tick } from './clock';

registerWidget({
  type: 'desktop-clock',
  name: 'Big clock',
  description: 'A large clock for the desktop.',
  icon: 'clock',
  containers: ['desktop'],
  defaults: { hour24: true, seconds: false, date: true },
  defaultSize: { w: 320, h: 120 },
  settings: {
    hour24: { label: '24-hour clock', type: 'boolean' },
    seconds: { label: 'Show seconds', type: 'boolean' },
    date: { label: 'Show the date', type: 'boolean' },
  },
  create(ctx) {
    const time = h('div', { class: 'dclock-time mono' });
    const date = h('div', { class: 'dclock-date' });
    const el = h('div', { class: 'dw-body dclock' }, time, date);
    let cfg = ctx.config;
    const render = () => {
      const d = new Date();
      const t = formatTime(d, !!cfg.hour24, !!cfg.seconds);
      time.textContent = t;
      date.textContent = `${formatDate(d)} · ${d.getFullYear()}`;
      date.hidden = !cfg.date;
      el.title = `${pad2(d.getHours())}:${pad2(d.getMinutes())}`;
    };
    tick(el, () => !!cfg.seconds, render);
    return {
      el,
      update(c) {
        cfg = c;
        render();
      },
    };
  },
});
