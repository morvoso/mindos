import { h } from '../dom';
import { icon } from '../icons';
import { formatTime } from '../widgets/clock';
import type { PopupContent, PopupCtx } from './shared';

const MONTHS = ['January', 'February', 'March', 'April', 'May', 'June', 'July', 'August', 'September', 'October', 'November', 'December'];
const DAYS = ['Sunday', 'Monday', 'Tuesday', 'Wednesday', 'Thursday', 'Friday', 'Saturday'];
const WEEK = ['SU', 'MO', 'TU', 'WE', 'TH', 'FR', 'SA'];

export function calendarPopup(ctx: PopupCtx): PopupContent {
  const now = new Date();
  let year = now.getFullYear();
  let month = now.getMonth();
  const hour24 = ctx.arg.hour24 !== false;
  const suffix = ctx.arg.suffix !== false;
  // Which weekday the grid starts on (0 = Sunday, 1 = Monday).
  const start = ctx.arg.weekStart === 'sunday' ? 0 : 1;

  const time = h('div', { class: 'cal-time mono' }, formatTime(now, hour24, false, { suffix }));
  const today = h('div', { class: 'cal-today' }, `${DAYS[now.getDay()]}, ${now.getDate()} ${MONTHS[now.getMonth()]} ${now.getFullYear()}`);
  const title = h('span', { class: 'cal-month' });
  const grid = h('div', { class: 'cal-grid' });
  const head = h('div', { class: 'cal-head' }, ...WEEK.map((_, i) => h('span', { class: 'cal-dow' }, WEEK[(i + start) % 7])));
  const prev = h('button', { class: 'tool', title: 'Previous month' }, icon('chevron-left', 14));
  const next = h('button', { class: 'tool', title: 'Next month' }, icon('chevron-right', 14));
  const back = h('button', { class: 'tool', title: 'Today' }, icon('calendar', 14));

  const render = () => {
    title.textContent = `${MONTHS[month]} ${year}`;
    const first = new Date(year, month, 1);
    const offset = (first.getDay() - start + 7) % 7;
    const daysIn = new Date(year, month + 1, 0).getDate();
    const prevDays = new Date(year, month, 0).getDate();
    grid.replaceChildren();
    for (let i = 0; i < 42; i++) {
      const d = i - offset + 1;
      let cls = 'cal-day';
      let n = d;
      if (d < 1) {
        n = prevDays + d;
        cls += ' other';
      } else if (d > daysIn) {
        n = d - daysIn;
        cls += ' other';
      }
      const isToday = d >= 1 && d <= daysIn && year === now.getFullYear() && month === now.getMonth() && d === now.getDate();
      if (isToday) cls += ' today';
      const dow = (i + start) % 7;
      if (dow === 0 || dow === 6) cls += ' weekend';
      grid.appendChild(h('span', { class: cls }, String(n)));
    }
  };
  prev.addEventListener('click', () => {
    month -= 1;
    if (month < 0) {
      month = 11;
      year -= 1;
    }
    render();
  });
  next.addEventListener('click', () => {
    month += 1;
    if (month > 11) {
      month = 0;
      year += 1;
    }
    render();
  });
  back.addEventListener('click', () => {
    year = now.getFullYear();
    month = now.getMonth();
    render();
  });
  render();
  const el = h(
    'div',
    { class: 'pop-body cal' },
    h('div', { class: 'cal-top' }, time, today),
    h('div', { class: 'cal-nav' }, title, h('span', { class: 'cal-nav-btns' }, prev, back, next)),
    head,
    grid,
  );
  return { el, w: 312 };
}
