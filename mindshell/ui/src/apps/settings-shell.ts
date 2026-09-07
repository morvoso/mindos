// Settings › Desktop (panels, shortcuts, shell config) and › About.

import * as bridge from '../bridge';
import { h } from '../dom';
import { icon } from '../icons';
import { store } from '../state';
import { MODES } from '../widgets/layout-mode';
import { card, notice, pageHeader, row, selectBox } from './shared';

const SHORTCUTS: [string, string][] = [
  ['Super', 'Open the Mind bar (tap)'],
  ['Super + Space', 'Open the Mind bar'],
  ['Super + Return', 'Terminal'],
  ['Super + Q', 'Close the window'],
  ['Super + F', 'Full screen'],
  ['Super + M', 'Maximise'],
  ['Super + W', 'Overview of all windows'],
  ['Super + Tab / Alt + Tab', 'Switch windows'],
  ['Super + T', 'Next window layout (floating → tiles → columns)'],
  ['Super + Shift + F', 'Float / tile the window'],
  ['Super + ← ↑ → ↓', 'Focus the window in that direction'],
  ['Super + Shift + ← ↑ → ↓', 'Move the window'],
  ['Super + R', 'Cycle the column width (columns layout)'],
  ['Super + 1 … 9', 'Focus display 1 … 9'],
  ['Super + Shift + E', 'Log out of the desktop'],
];

export function shellPage(el: HTMLElement): () => void {
  const note = notice();
  const cfg = store.state.config;
  const resetBtn = h('button', { class: 'btn danger' }, icon('refresh', 14), 'Reset the layout');
  let armed = false;
  resetBtn.addEventListener('click', () => {
    if (!armed) {
      armed = true;
      resetBtn.classList.add('armed');
      resetBtn.lastChild!.textContent = 'Confirm reset';
      setTimeout(() => {
        armed = false;
        resetBtn.classList.remove('armed');
        resetBtn.lastChild!.textContent = 'Reset the layout';
      }, 3000);
      return;
    }
    bridge
      .call('layout.reset')
      .then(() => note.show('The panels and widgets are back to the defaults.', 'ok'))
      .catch((e) => note.show(String(e instanceof Error ? e.message : e), 'error'));
  });

  const modeSel = selectBox(MODES.map((m) => ({ value: m.name, label: `${m.label} (${m.like})` })), store.layoutMode?.mode ?? 'floating', (v) => bridge.send('wm.setLayoutMode', { mode: v }));
  store.bind(modeSel, 'layoutMode', () => {
    if (store.layoutMode && document.activeElement !== modeSel) modeSel.value = store.layoutMode.mode;
  });
  if (!store.layoutMode) void store.fetchLayoutMode();

  const table = h('table', { class: 'keys' }, ...SHORTCUTS.map(([k, d]) => h('tr', {}, h('td', { class: 'mono key' }, k), h('td', {}, d))));

  el.append(
    pageHeader('Desktop', 'Panels, the dock, windows and shortcuts.'),
    note.el,
    card(
      'Windows',
      row('Layout', 'How windows are arranged. Also next to the clock and on Super+T.', modeSel),
    ),
    card(
      'Panels and the dock',
      h('p', { class: 'card-help' }, 'The bar is a panel. Right-click any widget on it for its settings (the clock\u2019s 12/24-hour format, what the task bar shows, and so on); right-click the desktop and choose Edit desktop to move panels, add widgets or make a new one.'),
      h('div', { class: 'card-actions' }, h('span', { class: 'strip-gap' }), resetBtn),
    ),
    card('Keyboard shortcuts', table),
    card(
      'Shell',
      row('Icon theme', 'From /etc/mindos/shell.toml or ~/.config/mindos/shell.toml.', h('span', { class: 'mono' }, cfg.icon_theme ?? 'default')),
      row('Terminal', null, h('span', { class: 'mono' }, cfg.terminal ?? 'foot')),
      row('Hardware acceleration', null, h('span', { class: 'mono' }, cfg.hardware_acceleration ?? 'auto')),
    ),
  );
  return () => undefined;
}

export function aboutPage(el: HTMLElement): () => void {
  const s = store.state;
  el.append(
    pageHeader('About'),
    card(
      null,
      h('div', { class: 'about-brand' }, h('span', { class: 'about-mark' }, icon('mind', 34)), h('div', {}, h('div', { class: 'about-name' }, 'MINDOS'), h('div', { class: 'about-sub' }, 'GAMING · DEV · AN ARCH-BASED DESKTOP WITH A MIND OF ITS OWN'))),
      row('Shell', null, h('span', { class: 'mono' }, `mindshell ${s.version ?? ''}`.trim())),
      row('Compositor', null, h('span', { class: 'mono' }, 'mindwm')),
      row('Assistant', null, h('span', { class: 'mono' }, s.mind?.model ? `Mind · ${s.mind.model}` : 'Mind')),
      row('Machine', null, h('span', { class: 'mono' }, `${s.user}@${s.host}`)),
    ),
    card(
      'Credits',
      h('p', { class: 'card-help' }, 'Built on Arch Linux, the Linux kernel, Smithay, GTK and WebKitGTK. Fonts: Inter, JetBrains Mono and Orbitron (SIL Open Font Licence). Models by Qwen (Apache-2.0) via Unsloth.'),
    ),
  );
  return () => undefined;
}
