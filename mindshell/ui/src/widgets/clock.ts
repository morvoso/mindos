import { h, pad2 } from '../dom';
import { registerWidget, type SettingSpec } from './registry';
import { panelItem } from './common';
import type { Config } from '../types';

const DAYS = ['Sun', 'Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat'];
const DAYS_LONG = ['Sunday', 'Monday', 'Tuesday', 'Wednesday', 'Thursday', 'Friday', 'Saturday'];
const MONTHS = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec'];
const MONTHS_LONG = ['January', 'February', 'March', 'April', 'May', 'June', 'July', 'August', 'September', 'October', 'November', 'December'];

export type DateFormat = 'short' | 'long' | 'numeric' | 'iso' | 'weekday';

export interface TimeOptions {
  hour24: boolean;
  seconds: boolean;
  /** 12-hour: append AM/PM (default true). */
  suffix?: boolean;
  /** Pad a one-digit hour (12-hour clocks show 9:05 unless this is set). */
  leadingZero?: boolean;
}

export function formatTime(d: Date, hour24: boolean, seconds: boolean, opts: Partial<TimeOptions> = {}): string {
  let hh = d.getHours();
  let suffix = '';
  if (!hour24) {
    if (opts.suffix !== false) suffix = hh >= 12 ? ' PM' : ' AM';
    hh = hh % 12 || 12;
  }
  const hour = hour24 || opts.leadingZero ? pad2(hh) : String(hh);
  const base = `${hour}:${pad2(d.getMinutes())}`;
  return (seconds ? `${base}:${pad2(d.getSeconds())}` : base) + suffix;
}

export function formatDate(d: Date, format: DateFormat = 'short'): string {
  switch (format) {
    case 'long':
      return `${DAYS_LONG[d.getDay()]}, ${d.getDate()} ${MONTHS_LONG[d.getMonth()]}`;
    case 'numeric':
      return `${pad2(d.getDate())}/${pad2(d.getMonth() + 1)}/${d.getFullYear()}`;
    case 'iso':
      return `${d.getFullYear()}-${pad2(d.getMonth() + 1)}-${pad2(d.getDate())}`;
    case 'weekday':
      return DAYS_LONG[d.getDay()];
    default:
      return `${DAYS[d.getDay()]} ${d.getDate()} ${MONTHS[d.getMonth()]}`;
  }
}

export const DATE_FORMATS: { value: DateFormat; label: string }[] = [
  { value: 'short', label: 'Sun 6 Sep' },
  { value: 'long', label: 'Sunday, 6 September' },
  { value: 'numeric', label: '06/09/2026' },
  { value: 'iso', label: '2026-09-06' },
  { value: 'weekday', label: 'Sunday' },
];

/** The time settings the panel clock and the desktop clock share. */
export function timeSettings(): Record<string, SettingSpec> {
  return {
    hour24: { label: 'Time format', type: 'enum', segmented: true, options: [{ value: false, label: '12-hour' }, { value: true, label: '24-hour' }] },
    suffix: { label: 'Show AM / PM', type: 'boolean', when: (c) => !c.hour24 },
    leadingZero: { label: 'Leading zero', type: 'boolean', help: '09:05 instead of 9:05', when: (c) => !c.hour24 },
    seconds: { label: 'Show seconds', type: 'boolean' },
    date: { label: 'Show the date', type: 'boolean' },
    dateFormat: { label: 'Date format', type: 'enum', options: DATE_FORMATS, when: (c) => !!c.date },
  };
}

export function timeOptions(cfg: Config): TimeOptions {
  return { hour24: !!cfg.hour24, seconds: !!cfg.seconds, suffix: cfg.suffix !== false, leadingZero: !!cfg.leadingZero };
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
  defaults: { hour24: false, suffix: true, leadingZero: false, seconds: false, date: true, dateFormat: 'short', stack: false, size: 'normal', weekStart: 'monday' },
  settings: {
    ...timeSettings(),
    stack: { label: 'Date under the time', type: 'boolean', help: 'Two lines, like Windows', when: (c) => !!c.date },
    size: { label: 'Text size', type: 'enum', segmented: true, options: [{ value: 'small', label: 'S' }, { value: 'normal', label: 'M' }, { value: 'large', label: 'L' }] },
    weekStart: { label: 'Calendar weeks start on', type: 'enum', segmented: true, options: [{ value: 'monday', label: 'Monday' }, { value: 'sunday', label: 'Sunday' }] },
  },
  create(ctx) {
    const time = h('span', { class: 'clock-time mono' });
    const date = h('span', { class: 'clock-date' });
    const el = panelItem(ctx, 'w-clock');
    el.append(time, date);
    let cfg = ctx.config;
    const render = () => {
      const d = new Date();
      const o = timeOptions(cfg);
      el.classList.toggle('stack', !!cfg.stack && !!cfg.date && !ctx.panel?.vertical);
      el.dataset.size = String(cfg.size || 'normal');
      if (ctx.panel?.vertical) {
        const hh = o.hour24 ? d.getHours() : d.getHours() % 12 || 12;
        time.textContent = `${pad2(hh)}\n${pad2(d.getMinutes())}`;
        date.hidden = true;
      } else {
        time.textContent = formatTime(d, o.hour24, o.seconds, o);
        date.textContent = formatDate(d, cfg.dateFormat as DateFormat);
        date.hidden = !cfg.date;
      }
      el.title = `${formatDate(d, 'long')} ${d.getFullYear()} · ${formatTime(d, o.hour24, false, o)}`;
    };
    tick(el, () => !!cfg.seconds, render);
    el.addEventListener('click', () => ctx.togglePopup('calendar', { hour24: !!cfg.hour24, suffix: cfg.suffix !== false, weekStart: cfg.weekStart || 'monday' }, { anchor: ctx.anchorOf(el) }));
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
