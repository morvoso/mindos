// Settings › Performance: the mode, what it does, and what happens while a
// game runs (GameMode hooks into mindos-perf).

import { every, h } from '../dom';
import { icon } from '../icons';
import { PERF_MODES, perfRefresh, perfSubscribe, perfSwitch, setPerfConfig } from '../perf';
import type { PerfStatus } from '../types';
import { card, notice, pageHeader, pill, row, selectBox, toggle } from './shared';

export function performancePage(el: HTMLElement): () => void {
  const note = notice();
  const fail = (e: unknown) => note.show(`mindos-perf: ${e instanceof Error ? e.message : String(e)}`, 'error');
  let status: PerfStatus | undefined;
  let busy = false;

  // ----- mode ---------------------------------------------------------------
  const modes = h('div', { class: 'perf-grid', role: 'group', 'aria-label': 'System performance mode' });
  const buttons = new Map<string, HTMLButtonElement>();
  for (const m of PERF_MODES) {
    const b = h('button', { class: `perf-card perf-${m.mode}`, title: m.detail, 'aria-pressed': 'false' }, h('span', { class: 'perf-ic' }, icon(m.icon, 26)), h('span', { class: 'perf-name' }, m.label), h('span', { class: 'perf-blurb' }, m.blurb));
    b.addEventListener('click', () => {
      if (busy) return;
      setBusy(true);
      note.show('Applying mode…');
      perfSwitch(m.mode)
        .then((msg) => note.show(msg, 'ok'))
        .catch(fail)
        .finally(() => setBusy(false));
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
    h('p', { class: 'card-help' }, 'Games launched with GameMode use your gaming mode and restore your desktop mode when they finish. In Steam, set the game’s launch options to ', h('code', {}, 'gamemoderun %command%'), '. In Lutris, enable GameMode in System options.'),
    row('Mode while a game runs', 'Applied when a game starts, regardless of the current mode.', gameSel),
    row('Free the Mind’s GPU memory', 'Unloads the local model during games. Asking the Mind a question loads it again.', sleepToggle),
  );

  // ----- performance mode details --------------------------------------------
  const scxSel = selectBox([{ value: 'scx_lavd', label: 'scx_lavd (games, latency)' }, { value: 'scx_bpfland', label: 'scx_bpfland (interactive)' }, { value: 'scx_rusty', label: 'scx_rusty (throughput)' }, { value: '', label: 'Kernel default (EEVDF + BORE)' }], 'scx_lavd', (v) => setConfig('SCX_SCHEDULER', v));
  const plSel = selectBox([{ value: 'default', label: 'Card default' }, { value: 'max', label: 'Maximum the card allows' }], 'default', (v) => setConfig('NVIDIA_POWER_LIMIT', v));
  const tuneCard = card(
    'Performance mode',
    row('Scheduler', 'Used the next time performance mode starts. Compare frame times in your games before changing the default.', scxSel),
    row('NVIDIA power limit', 'Raises the power limit in performance mode. The card draws only what its workload requires.', plSel),
  );

  // ----- status table -----------------------------------------------------------
  const statusBody = h('div', { class: 'kv' });
  const statusCard = card('Current state', statusBody);

  const advanced = h('details', { class: 'settings-advanced' }, h('summary', {}, 'Advanced tuning', h('span', {}, 'Scheduler and GPU power')), tuneCard, statusCard);
  el.append(pageHeader('Performance', 'Choose how your computer balances speed, power and noise.'), note.el, modeCard, gameCard, advanced);

  const sleepInput = sleepToggle.querySelector('input')!;
  const setBusy = (value: boolean) => {
    busy = value;
    modes.classList.toggle('busy', value);
    el.setAttribute('aria-busy', String(value));
    for (const control of [...buttons.values(), gameSel, sleepInput, scxSel, plSel]) control.disabled = value || !status;
    plSel.disabled ||= !status?.nvidia;
  };

  const setConfig = (key: string, value: string) => {
    if (busy) return;
    setBusy(true);
    note.show('Saving…');
    setPerfConfig(key, value)
      .then(() => {
        note.show(key === 'NVIDIA_POWER_LIMIT' && status?.effective === 'performance' ? 'Power limit applied.' : 'Saved. Used at the next mode or game transition.', 'ok');
      })
      .catch(fail)
      .finally(async () => { await perfRefresh(true); setBusy(false); });
  };

  const kv = (k: string, v: string | undefined | null) => (v ? [h('div', { class: 'kv-k' }, k), h('div', { class: 'kv-v mono' }, v)] : []);

  const render = (s: PerfStatus | undefined) => {
    status = s;
    for (const [mode, b] of buttons) {
      b.classList.toggle('on', s?.mode === mode);
      b.classList.toggle('effective', !!s && s.effective === mode && s.effective !== s.mode);
      b.setAttribute('aria-pressed', String(s?.mode === mode));
    }
    setBusy(busy);
    if (!s) {
      nowLine.textContent = 'mindos-perf is not answering; is mindos-base installed?';
      statusBody.replaceChildren();
      return;
    }
    nowLine.replaceChildren(
      s.game > 0 ? pill(`Game running · ${s.effective}`, 'accent') : pill(`${s.effective || s.mode} in effect`, 'ok'),
      ' ',
      s.game > 0 ? `GameMode is active. ${s.effective} now; ${s.mode} after gaming.` : 'Changes apply now and are remembered after reboot.',
    );
    const g = gameSel;
    if (document.activeElement !== g) g.value = s.gameMode ?? '';
    const st = sleepToggle.querySelector('input') as HTMLInputElement;
    if (document.activeElement !== st) st.checked = !!s.mindSleeps;
    const setChoice = (select: HTMLSelectElement, value: string, label: string) => {
      if (![...select.options].some((option) => option.value === value)) select.add(h('option', { value }, label));
      select.value = value;
    };
    if (document.activeElement !== scxSel || !busy) setChoice(scxSel, s.scx || '', s.scx);
    if (document.activeElement !== plSel || !busy) setChoice(plSel, s.powerLimitPolicy ?? 'default', `${s.powerLimitPolicy} W (custom)`);
    statusBody.replaceChildren(
      ...kv('CPU', s.cpu),
      ...kv('Driver', s.driver),
      ...kv('Governor', s.governor && s.epp ? `${s.governor} · EPP ${s.epp}` : s.governor),
      ...kv('Boost', s.boost === null ? undefined : s.boost ? 'on' : 'off'),
      ...kv('Platform profile', s.platformProfile),
      ...kv('Running scheduler', s.scheduler),
      ...kv('Huge pages', s.thp),
      ...kv('GPU', s.gpu),
      ...kv('Persistence', s.nvidia ? s.persistence ? 'on' : 'off' : undefined),
      ...kv('Applied power limit', s.nvidia && s.powerLimit ? `${s.powerLimit} W` : undefined),
    );
  };
  setBusy(false);
  const unsubscribe = perfSubscribe(el, render);
  void perfRefresh(true);
  const stop = every(el, 5000, () => void perfRefresh());
  return () => { stop(); unsubscribe(); };
}
