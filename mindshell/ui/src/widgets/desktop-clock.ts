import { h } from '../dom';
import { registerWidget } from './registry';
import { formatDate, formatTime, tick, timeOptions, timeSettings, type DateFormat } from './clock';

registerWidget({
  type: 'desktop-clock',
  name: 'Big clock',
  description: 'A large clock for the desktop.',
  icon: 'clock',
  containers: ['desktop'],
  defaults: { hour24: false, suffix: true, leadingZero: false, seconds: false, date: true, dateFormat: 'short', year: true, align: 'left', size: 64, glow: true },
  defaultSize: { w: 320, h: 120 },
  settings: {
    ...timeSettings(),
    year: { label: 'Show the year', type: 'boolean', when: (c) => !!c.date && c.dateFormat !== 'numeric' && c.dateFormat !== 'iso' },
    size: { label: 'Text size', type: 'number', min: 24, max: 160, step: 2, unit: 'px' },
    align: { label: 'Alignment', type: 'enum', segmented: true, options: [{ value: 'left', label: 'Left' }, { value: 'center', label: 'Centre' }, { value: 'right', label: 'Right' }] },
    glow: { label: 'Glow', type: 'boolean' },
  },
  create(ctx) {
    const time = h('div', { class: 'dclock-time mono' });
    const date = h('div', { class: 'dclock-date' });
    const el = h('div', { class: 'dw-body dclock' }, time, date);
    let cfg = ctx.config;
    const render = () => {
      const d = new Date();
      const o = timeOptions(cfg);
      time.textContent = formatTime(d, o.hour24, o.seconds, o);
      const f = (cfg.dateFormat as DateFormat) || 'short';
      const showYear = !!cfg.year && f !== 'numeric' && f !== 'iso';
      date.textContent = showYear ? `${formatDate(d, f)} · ${d.getFullYear()}` : formatDate(d, f);
      date.hidden = !cfg.date;
      const size = Math.max(16, Number(cfg.size) || 64);
      time.style.fontSize = `${size}px`;
      date.style.fontSize = `${Math.max(10, Math.round(size * 0.21))}px`;
      el.style.textAlign = String(cfg.align || 'left');
      el.classList.toggle('no-glow', cfg.glow === false);
      el.title = `${formatDate(d, 'long')} ${d.getFullYear()}`;
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
