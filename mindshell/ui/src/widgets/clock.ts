import { h, pad2 } from '../dom';
import { registerWidget } from './registry';
import { panelItem } from './common';

const DAYS = ['Sun', 'Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat'];
const MONTHS = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec'];

export function formatTime(d: Date, hour24: boolean, seconds: boolean): string {
  let hh = d.getHours();
  let suffix = '';
  if (!hour24) {
    suffix = hh >= 12 ? ' PM' : ' AM';
    hh = hh % 12 || 12;
  }
  const base = `${hour24 ? pad2(hh) : hh}:${pad2(d.getMinutes())}`;
  return (seconds ? `${base}:${pad2(d.getSeconds())}` : base) + suffix;
}

export function formatDate(d: Date): string {
  return `${DAYS[d.getDay()]} ${d.getDate()} ${MONTHS[d.getMonth()]}`;
}

/** Call `fn` now and at every second/minute boundary while `el` is in the document. */
export function tick(el: Element, perSecond: () => boolean, fn: () => void): void {
  let timer: ReturnType<typeof setTimeout> | undefined;
  const loop = () => {
    if (!el.isConnected && timer !== undefined) return;
    fn();
    const now = Date.now();
    const period = perSecond() ? 1000 : 60000;
    timer = setTimeout(loop, period - (now % period) + 5);
  };
  loop();
}

registerWidget({
  type: 'clock',
  name: 'Clock',
  description: 'Time and date. Click for the calendar.',
  icon: 'clock',
  containers: ['panel'],
  defaults: { seconds: false, date: true, hour24: true },
  settings: {
    hour24: { label: '24-hour clock', type: 'boolean' },
    seconds: { label: 'Show seconds', type: 'boolean' },
    date: { label: 'Show the date', type: 'boolean' },
  },
  create(ctx) {
    const time = h('span', { class: 'clock-time mono' });
    const date = h('span', { class: 'clock-date' });
    const el = panelItem(ctx, 'w-clock');
    el.append(time, date);
    let cfg = ctx.config;
    const render = () => {
      const d = new Date();
      if (ctx.panel?.vertical) {
        time.textContent = `${pad2(d.getHours())}\n${pad2(d.getMinutes())}`;
        date.hidden = true;
      } else {
        time.textContent = formatTime(d, !!cfg.hour24, !!cfg.seconds);
        date.textContent = formatDate(d);
        date.hidden = !cfg.date;
      }
      el.title = d.toLocaleString();
    };
    tick(el, () => !!cfg.seconds, render);
    el.addEventListener('click', () => ctx.togglePopup('calendar', {}, { anchor: ctx.anchorOf(el) }));
    ctx.store.bind(el, 'popups', () => el.classList.toggle('open', ctx.store.popups.has('calendar')));
    return {
      el,
      update(c) {
        cfg = c;
        render();
      },
    };
  },
});
