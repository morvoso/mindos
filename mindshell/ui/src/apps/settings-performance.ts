// Settings › Performance: the mode, what it does, and what happens while a
// game runs (GameMode hooks into mindos-perf).

import { h } from '../dom';
import { icon } from '../icons';
import { PERF_MODES, perfRefresh, perfSubscribe, perfSwitch, setPerfConfig } from '../perf';
import type { PerfStatus } from '../types';
import { card, notice, pageHeader, pill, row, selectBox, toggle } from './shared';

export function performancePage(el: HTMLElement): () => void {
  const note = notice();
  const fail = (e: unknown) => note.show(`mindos-perf: ${e instanceof Error ? e.message : String(e)}`, 'error');
  let status: PerfStatus | undefined;

  // ----- mode ---------------------------------------------------------------
  const modes = h('div', { class: 'perf-grid' });
  const buttons = new Map<string, HTMLElement>();
  for (const m of PERF_MODES) {
    const b = h('button', { class: `perf-card perf-${m.mode}` }, h('span', { class: 'perf-ic' }, icon(m.icon, 26)), h('span', { class: 'perf-name' }, m.label), h('span', { class: 'perf-blurb' }, m.blurb), h('span', { class: 'perf-detail' }, m.detail));
    b.addEventListener('click', () => {
      modes.classList.add('busy');
      perfSwitch(m.mode)
        .then((msg) => note.show(msg, 'ok'))
        .catch(fail)
        .finally(() => modes.classList.remove('busy'));
    });
    buttons.set(m.mode, b);
    modes.appendChild(b);
  }
  const nowLine = h('div', { class: 'row-help perf-now' });
  const modeCard = card('Mode', modes, nowLine);

  // ----- while gaming ---------------------------------------------------------
  const gameSel = selectBox([{ value: 'performance', label: 'Performance' }, { value: 'balanced', label: 'Balanced' }, { value: 'quiet', label: 'Quiet' }, { value: '', label: 'Leave as is' }], 'performance', (v) => setConfig('GAME_MODE', v));
  const sleepToggle = toggle(true, (v) => setConfig('MIND_SLEEPS_WHILE_GAMING', v ? '1' : '0'));
  const gameCard = card(
    'While a game runs',
    h('p', { class: 'card-help' }, 'Steam and Lutris start games through GameMode. MindOS switches the performance mode when a game starts and restores the previous mode when it ends.'),
    row('Mode while a game runs', 'Applied when a game starts, regardless of the current mode.', gameSel),
    row('Suspend the Mind during games', 'Unloads the language model from the GPU so the full video memory is available to the game. The Mind reloads when it is next used or when the game ends.', sleepToggle),
  );

  // ----- performance mode details --------------------------------------------
  const scxSel = selectBox([{ value: 'scx_lavd', label: 'scx_lavd (games, latency)' }, { value: 'scx_bpfland', label: 'scx_bpfland (interactive)' }, { value: 'scx_rusty', label: 'scx_rusty (throughput)' }, { value: '', label: 'Kernel default (EEVDF + BORE)' }], 'scx_lavd', (v) => setConfig('SCX_SCHEDULER', v));
  const plSel = selectBox([{ value: 'default', label: 'Card default' }, { value: 'max', label: 'Maximum the card allows' }], 'default', (v) => setConfig('NVIDIA_POWER_LIMIT', v));
  const tuneCard = card(
    'Performance mode',
    row('Scheduler', 'The sched_ext scheduler loaded in performance mode. scx_lavd is designed for games: it keeps game threads on the fastest cores and maintains consistent frame pacing.', scxSel),
    row('NVIDIA power limit', 'Raises the power limit in performance mode. The card draws only what its workload requires.', plSel),
  );

  // ----- status table -----------------------------------------------------------
  const statusBody = h('div', { class: 'kv' });
  const statusCard = card('Current state', statusBody);

  el.append(pageHeader('Performance', 'Three system-wide modes covering the CPU governor and boost, the scheduler, memory and the GPU. GameMode switches modes automatically when a game starts.'), note.el, modeCard, gameCard, tuneCard, statusCard);

  const setConfig = (key: string, value: string) => {
    setPerfConfig(key, value)
      .then(() => {
        note.show('Saved.', 'ok');
        return perfRefresh(true);
      })
      .catch(fail);
  };

  const kv = (k: string, v: string | undefined | null) => (v ? [h('div', { class: 'kv-k' }, k), h('div', { class: 'kv-v mono' }, v)] : []);

  const render = (s: PerfStatus | undefined) => {
    status = s;
    for (const [mode, b] of buttons) {
      b.classList.toggle('on', s?.mode === mode);
      b.classList.toggle('effective', !!s && s.effective === mode && s.effective !== s.mode);
    }
    if (!s) {
      nowLine.textContent = 'mindos-perf is not answering; is mindos-base installed?';
      statusBody.replaceChildren();
      return;
    }
    nowLine.replaceChildren(
      s.game > 0 ? pill(`Game running · ${s.effective}`, 'accent') : pill(`${s.effective || s.mode} in effect`, 'ok'),
      ' ',
      s.game > 0 ? `${s.game} game${s.game > 1 ? 's' : ''} running: ${s.effective} mode until ${s.game > 1 ? 'they end' : 'it ends'}, then ${s.mode} is restored.` : `Applied at boot and whenever the mode is changed.`,
    );
    const g = gameSel;
    if (document.activeElement !== g) g.value = s.gameMode ?? '';
    const st = sleepToggle.querySelector('input') as HTMLInputElement;
    if (document.activeElement !== st) st.checked = !!s.mindSleeps;
    if (document.activeElement !== scxSel) scxSel.value = s.scheduler.startsWith('scx_') ? s.scheduler : (s.scx || '');
    if (document.activeElement !== plSel && (s.powerLimit === 'default' || s.powerLimit === 'max')) plSel.value = s.powerLimit;
    statusBody.replaceChildren(
      ...kv('CPU', s.cpu),
      ...kv('Driver', s.driver),
      ...kv('Governor', s.governor && s.epp ? `${s.governor} · EPP ${s.epp}` : s.governor),
      ...kv('Boost', s.boost === null ? undefined : s.boost ? 'on' : 'off'),
      ...kv('Platform profile', s.platformProfile),
      ...kv('Scheduler', s.scx ? `${s.scx} (sched_ext)` : s.scheduler),
      ...kv('Huge pages', s.thp),
      ...kv('GPU', s.gpu ? `${s.gpu}${s.nvidia ? ' · persistence on' : ''}${s.powerLimit && s.powerLimit !== 'default' ? ' · limit ' + s.powerLimit : ''}` : undefined),
    );
  };
  perfSubscribe(el, render);
  render(undefined);
  void perfRefresh(true);
  const timer = setInterval(() => void perfRefresh(), 5000);
  return () => clearInterval(timer);
}
