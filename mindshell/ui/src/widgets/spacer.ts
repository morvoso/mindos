import { h } from '../dom';
import { registerWidget } from './registry';

registerWidget({
  type: 'spacer',
  name: 'Spacer',
  description: 'A flexible gap that pushes the widgets after it to the far end, or a fixed gap.',
  icon: 'minus',
  containers: ['panel'],
  defaults: { expand: true, size: 16 },
  settings: {
    expand: { label: 'Expand to fill free space', type: 'boolean' },
    size: { label: 'Fixed size', type: 'number', min: 4, max: 400, step: 4, unit: 'px', when: (c) => !c.expand },
  },
  create(ctx) {
    const el = h('div', { class: 'w w-spacer' });
    const apply = (cfg = ctx.config) => {
      el.classList.toggle('expand', !!cfg.expand);
      el.style.flexBasis = cfg.expand ? '0' : `${Number(cfg.size) || 16}px`;
    };
    apply();
    return { el, update: apply };
  },
});
