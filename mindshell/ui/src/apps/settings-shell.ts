// Settings › Desktop (panels, shortcuts, shell config) and › About.

import * as bridge from '../bridge';
import { h } from '../dom';
import { icon } from '../icons';
import { store } from '../state';
import type { PointerState } from '../types';
import { MODES } from '../widgets/layout-mode';
import { card, notice, pageHeader, row, selectBox } from './shared';

const SHORTCUTS: [string, string][] = [
  ['Volume / mute keys', 'Adjust output in 5% steps; hold to repeat (up to 100%)'],
  ['Microphone mute', 'Mute or unmute the default microphone'],
  ['Brightness keys', 'Adjust the built-in display in 5% steps; hold to repeat'],
  ['Media keys', 'Play / pause, stop, next or previous track'],
  ['Super + Space', 'Open the Mind bar'],
  ['Super + Return', 'Terminal'],
  ['Print / Super + Shift + S', 'Select an area to save and copy'],
  ['Shift + Print', 'Save and copy all displays'],
  ['Alt + F4 / Super + Q', 'Close the window'],
  ['Super + F', 'Full screen'],
  ['Super + M', 'Maximise'],
  ['Super + W', 'Overview of all windows'],
  ['Alt + Tab / Super + Tab', 'Switch recent windows; hold Alt or Super to keep cycling'],
  ['Alt + Shift + Tab / Super + Shift + Tab', 'Switch backward; Escape returns to the original window'],
  ['Super + T', 'Next window layout (floating → tiles → columns)'],
  ['Super + Shift + F', 'Float / tile the window'],
  ['Super + ← ↑ → ↓', 'Focus the window in that direction'],
  ['Super + Shift + ← ↑ → ↓', 'Move the window'],
  ['Super + R', 'Cycle the column width (columns layout)'],
  ['Super + mouse wheel', 'Step through the windows (tiles and columns)'],
  ['Super + 1 … 9', 'Focus display 1 … 9'],
  ['Super + Shift + E', 'Log out of the desktop'],
];

const CURSOR_SIZES: [number, string][] = [
  [24, 'Small (24)'],
  [32, 'Medium (32)'],
  [48, 'Large (48)'],
  [64, 'Huge (64)'],
];

/** Settings › Desktop › Pointer: the cursor theme and its size. */
function pointerCard(note: ReturnType<typeof notice>): HTMLElement {
  const themeSel = selectBox([], '', () => {});
  const sizeSel = selectBox(CURSOR_SIZES.map(([v, label]) => ({ value: v, label })), 24, () => {});
  const body = h('div', {}, 
    row('Theme', 'The pointer shapes. MindOS is the animated one that matches the desktop.', themeSel),
    row('Size', 'Applications that read the size once at start (games, Qt) use it the next time they run.', sizeSel),
  );

  const apply = (change: { theme?: string; size?: number }) => {
    bridge
      .call<PointerState>('pointer.set', change)
      .then((p) => { fill(p); note.show('The pointer changed.', 'ok'); })
      .catch((e) => note.show(String(e instanceof Error ? e.message : e), 'error'));
  };
  themeSel.addEventListener('change', () => apply({ theme: themeSel.value }));
  sizeSel.addEventListener('change', () => apply({ size: Number(sizeSel.value) }));

  const fill = (p: PointerState) => {
    themeSel.replaceChildren(...p.themes.map((t) => h('option', { value: t }, t)));
    themeSel.value = p.theme;
    if (!themeSel.value) themeSel.append(h('option', { value: p.theme, selected: '' }, p.theme));
    sizeSel.value = String(p.size);
    if (sizeSel.value !== String(p.size)) {
      sizeSel.append(h('option', { value: String(p.size) }, String(p.size)));
      sizeSel.value = String(p.size);
    }
    themeSel.disabled = sizeSel.disabled = !p.writable;
  };
  bridge
    .call<PointerState>('pointer.get')
    .then(fill)
    .catch(() => body.prepend(h('p', { class: 'card-help' }, 'The pointer settings are unavailable.')));
  return card('Pointer', body);
}

export function shellPage(el: HTMLElement): () => void {
  const note = notice();
  const cfg = store.state.config;
  // The configured icon theme is shown here; the shell falls back safely if its package is absent.
  const themeName = h('span', { class: 'mono' }, cfg.icon_theme ?? 'default');
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
      .then(() => note.show('Panels and widgets have been reset to the defaults.', 'ok'))
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
      row('Layout', 'How windows are arranged. Also available next to the clock and with Super+T.', modeSel),
    ),
    card(
      'Panels and the dock',
      h('p', { class: 'card-help' }, 'The bar is a panel. Right-click a widget for its settings (for example the clock\u2019s 12/24-hour format or the task bar contents). Right-click the desktop and choose Edit desktop to move panels, add widgets or create a new panel.'),
      h('div', { class: 'card-actions' }, h('span', { class: 'strip-gap' }), resetBtn),
    ),
    pointerCard(note),
    card('Keyboard shortcuts', table),
    card(
      'Shell',
      row('Icon theme', 'The icon theme in use. Set icon_theme in ~/.config/mindos/shell.toml to choose another.', themeName),
      row('Terminal', null, h('span', { class: 'mono' }, cfg.terminal ?? 'kitty')),
      row('Hardware acceleration', null, h('span', { class: 'mono' }, cfg.hardware_acceleration ?? 'auto')),
    ),
  );
  return store.on('config', () => {
    themeName.textContent = store.state.config.icon_theme ?? 'default';
  });
}

export function aboutPage(el: HTMLElement): () => void {
  const s = store.state;
  el.append(
    pageHeader('About'),
    card(
      null,
      h('div', { class: 'about-brand' }, h('span', { class: 'about-mark' }, icon('mind', 34)), h('div', {}, h('div', { class: 'about-name' }, 'MINDOS'), h('div', { class: 'about-sub' }, 'GAMING · AN ARCH-BASED DESKTOP WITH A MIND OF ITS OWN'))),
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
