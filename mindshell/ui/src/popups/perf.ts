// The performance mode chooser: three modes, the current one lit, plus what
// GameMode is doing right now.

import { h } from '../dom';
import { icon } from '../icons';
import { PERF_MODES, perfRefresh, perfSubscribe, perfSwitch } from '../perf';
import type { PerfStatus } from '../types';
import { openApp } from '../apps/shared';
import type { PopupContent, PopupCtx } from './shared';

export function perfPopup(ctx: PopupCtx): PopupContent {
  const list = h('div', { class: 'perf-modes' });
  const foot = h('div', { class: 'pop-hint perf-foot' });
  const err = h('div', { class: 'pop-hint danger', hidden: true });
  const buttons = new Map<string, HTMLElement>();
  for (const m of PERF_MODES) {
    const b = h('button', { class: `perf-mode perf-${m.mode}` }, h('span', { class: 'perf-ic' }, icon(m.icon, 20)), h('span', { class: 'perf-text' }, h('span', { class: 'perf-name' }, m.label), h('span', { class: 'perf-blurb' }, m.blurb)), h('span', { class: 'perf-check' }, icon('check', 14)));
    b.addEventListener('click', () => {
      list.classList.add('busy');
      perfSwitch(m.mode)
        .then((msg) => {
          err.hidden = true;
          if (/stays in effect/.test(msg)) {
            err.textContent = msg;
            err.hidden = false;
          }
        })
        .catch((e) => {
          err.textContent = `Could not switch: ${e instanceof Error ? e.message : e}`;
          err.hidden = false;
        })
        .finally(() => {
          list.classList.remove('busy');
          ctx.relayout();
        });
    });
    buttons.set(m.mode, b);
    list.appendChild(b);
  }
  const render = (s: PerfStatus | undefined) => {
    for (const [mode, b] of buttons) {
      b.classList.toggle('on', s?.mode === mode);
      b.classList.toggle('effective', !!s && s.effective === mode && s.effective !== s.mode);
    }
    if (!s) {
      foot.textContent = 'mindos-perf is not answering.';
      return;
    }
    const bits: string[] = [];
    if (s.game > 0) bits.push(`A game is running: ${s.effective} mode until it ends${s.mindSleeps ? ', the Mind is asleep' : ''}.`);
    else if (s.gameMode) bits.push(`While a game runs: ${s.gameMode}${s.mindSleeps ? ', Mind sleeps' : ''}.`);
    if (s.scx) bits.push(`Scheduler ${s.scx}.`);
    if (s.governor) bits.push(`${s.governor}${s.epp ? ' / ' + s.epp : ''}.`);
    foot.textContent = bits.join(' ');
  };
  perfSubscribe(list, render);
  void perfRefresh(true);
  const head = h('div', { class: 'pop-head' }, h('span', { class: 'pop-title' }, 'PERFORMANCE'), h('span', { class: 'strip-gap' }), h('button', { class: 'btn small icon-btn', title: 'Settings › Performance', onclick: () => { openApp('settings', 'performance'); ctx.close(); } }, icon('gear', 14)));
  const el = h('div', { class: 'pop-body perf' }, head, list, err, foot);
  return { el, w: 340 };
}
