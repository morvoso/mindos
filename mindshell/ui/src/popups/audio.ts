import * as bridge from '../bridge';
import { h } from '../dom';
import { icon } from '../icons';
import { volumeIcon } from '../widgets/audio';
import type { AudioState } from '../types';
import type { PopupContent, PopupCtx } from './shared';

export function audioPopup(ctx: PopupCtx): PopupContent {
  const store = ctx.store;
  const mute = h('button', { class: 'tool big', title: 'Mute' });
  const slider = h('input', { type: 'range', min: 0, max: 100, step: 1, class: 'vol' }) as HTMLInputElement;
  const val = h('span', { class: 'mono vol-val' });
  const sink = h('div', { class: 'aud-sink' });
  const el = h('div', { class: 'pop-body aud' }, h('div', { class: 'pop-title' }, 'OUTPUT'), sink, h('div', { class: 'aud-row' }, mute, slider, val));

  const render = () => {
    const a = store.state.audio;
    mute.replaceChildren(icon(volumeIcon(a), 18));
    mute.classList.toggle('on', !!a?.muted);
    if (document.activeElement !== slider) slider.value = String(Math.round((a?.volume ?? 0) * 100));
    slider.style.setProperty('--fill', `${Math.round((a?.volume ?? 0) * 100)}%`);
    slider.disabled = !a;
    val.textContent = a ? (a.muted ? 'MUTE' : `${Math.round(a.volume * 100)}%`) : '--';
    sink.textContent = a?.sink || 'Default output';
    el.classList.toggle('muted', !!a?.muted);
  };
  slider.addEventListener('input', () => {
    const v = Number(slider.value) / 100;
    slider.style.setProperty('--fill', `${slider.value}%`);
    val.textContent = `${slider.value}%`;
    bridge.send('audio.set', { volume: v, muted: false });
    if (store.state.audio) {
      store.state.audio = { ...store.state.audio, volume: v, muted: false };
      store.emit('audio');
    }
  });
  mute.addEventListener('click', () => bridge.send('audio.toggleMute'));
  render();
  if (!store.state.audio) {
    bridge.call<AudioState>('audio.get').then((a) => {
      store.state.audio = a;
      store.emit('audio');
    }).catch(() => undefined);
  }
  store.bind(el, 'audio', render);
  return { el, w: 320, focus: () => slider.focus() };
}
