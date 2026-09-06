import { debounce, h } from '../dom';
import { registerWidget } from './registry';

registerWidget({
  type: 'desktop-notes',
  name: 'Notes',
  description: 'A scratchpad that lives on the desktop.',
  icon: 'note',
  containers: ['desktop'],
  defaults: { title: 'NOTES', text: '' },
  defaultSize: { w: 300, h: 220 },
  settings: { title: { label: 'Title', type: 'string' } },
  create(ctx) {
    const title = h('div', { class: 'dw-title' });
    const area = h('textarea', { class: 'notes-area', spellcheck: 'false', placeholder: 'Type here…' }) as HTMLTextAreaElement;
    const el = h('div', { class: 'dw-body notes' }, title, area);
    let cfg = ctx.config;
    const render = () => {
      title.textContent = String(cfg.title || 'NOTES');
      if (document.activeElement !== area) area.value = String(cfg.text ?? '');
    };
    const save = debounce(() => ctx.setConfig({ text: area.value }), 800);
    area.addEventListener('input', save);
    area.addEventListener('blur', () => {
      if (area.value !== String(cfg.text ?? '')) ctx.setConfig({ text: area.value });
    });
    render();
    return {
      el,
      update(c) {
        cfg = c;
        render();
      },
    };
  },
});
