import * as bridge from '../bridge';
import { clamp, h } from '../dom';
import { icon } from '../icons';
import { registerWidget } from './registry';
import { panelItem, pct } from './common';
import type { AudioState } from '../types';

export function volumeIcon(a: AudioState | undefined): string {
  if (!a || a.muted || a.volume <= 0) return 'volume-mute';
  return a.volume < 0.5 ? 'volume-low' : 'volume-high';
}

registerWidget({
  type: 'audio',
  name: 'Volume',
  description: 'Output volume. Scroll to adjust, middle-click to mute.',
  icon: 'volume-high',
  containers: ['panel'],
  defaults: { percent: true, step: 5, scroll: true, hideWhenMuted: false },
  settings: {
    percent: { label: 'Show the percentage', type: 'boolean' },
    scroll: { label: 'Scroll to change the volume', type: 'boolean' },
    step: { label: 'Scroll step', type: 'number', min: 1, max: 25, step: 1, unit: '%', when: (c) => !!c.scroll },
    hideWhenMuted: { label: 'Hide the percentage while muted', type: 'boolean', when: (c) => !!c.percent },
  },
  create(ctx) {
    const el = panelItem(ctx, 'w-audio', 'Volume');
    const ic = h('span', { class: 'w-ic' });
    const label = h('span', { class: 'w-val mono' });
    el.append(ic, label);
    let cfg = ctx.config;
    let current: string | undefined;
    const render = () => {
      const a = ctx.store.state.audio;
      const name = volumeIcon(a);
      if (name !== current) {
        current = name;
        ic.replaceChildren(icon(name, 18));
      }
      label.textContent = a ? (a.muted ? 'MUTE' : pct(a.volume * 100)) : '--';
      label.hidden = !cfg.percent || !!ctx.panel?.vertical || (!!cfg.hideWhenMuted && !!a?.muted);
      el.classList.toggle('muted', !!a?.muted);
      el.title = a ? `${a.sink || 'Output'} · ${a.muted ? 'muted' : pct(a.volume * 100)}` : 'Volume';
    };
    render();
    if (!ctx.store.state.audio) {
      bridge.call<AudioState>('audio.get').then((a) => {
        if (a) {
          ctx.store.state.audio = a;
          ctx.store.emit('audio');
        }
      }).catch(() => undefined);
    }
    ctx.store.bind(el, 'audio', render);
    ctx.store.bind(el, 'popups', () => el.classList.toggle('open', ctx.store.popups.has('audio')));
    el.addEventListener('click', () => ctx.togglePopup('audio', {}, { anchor: ctx.anchorOf(el) }));
    el.addEventListener('auxclick', (e) => {
      if (e.button === 1) bridge.send('audio.toggleMute');
    });
    el.addEventListener(
      'wheel',
      (e) => {
        if (!cfg.scroll) return;
        e.preventDefault();
        const a = ctx.store.state.audio;
        if (!a) return;
        const step = (Number(cfg.step) || 5) / 100;
        const v = clamp(a.volume - Math.sign(e.deltaY) * step, 0, 1);
        bridge.send('audio.set', { volume: v, muted: false });
        ctx.store.state.audio = { ...a, volume: v, muted: false };
        ctx.store.emit('audio');
      },
      { passive: false },
    );
    return {
      el,
      update(c) {
        cfg = c;
        render();
      },
    };
  },
});
