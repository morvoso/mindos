import * as bridge from '../bridge';
import { h } from '../dom';
import type { InputSettings, InputState, Prefs } from '../types';
import { card, notice, pageHeader, row, selectBox, toggle } from './shared';

export const INPUT_DEFAULTS: InputSettings = {
  keyboard_layout: '', keyboard_variant: '', keyboard_options: '', repeat_rate: 25, repeat_delay: 200,
  mouse_profile: 'default', mouse_speed: 0, mouse_left_handed: false, mouse_natural_scroll: false,
};

export function inputPage(el: HTMLElement): () => void {
  const note = notice();
  const form = h('fieldset', { class: 'input-settings-fields', disabled: true });
  let current = { ...INPUT_DEFAULTS };
  let alive = true;
  let busy = false;
  const devices = h('div', { class: 'card-help', 'aria-live': 'polite' }, 'Checking connected mice…');
  const layout = selectBox([
    ['', 'System default'], ['us', 'English (US)'], ['gb', 'English (UK)'], ['de', 'German'],
    ['fr', 'French'], ['es', 'Spanish'], ['it', 'Italian'], ['pt', 'Portuguese'],
    ['br', 'Portuguese (Brazil)'], ['se', 'Swedish'], ['no', 'Norwegian'], ['fi', 'Finnish'],
    ['pl', 'Polish'], ['cz', 'Czech'], ['jp', 'Japanese'], ['ru', 'Russian'],
    ['ua', 'Ukrainian'], ['__custom', 'Custom / multiple layouts'],
  ].map(([value, label]) => ({ value, label })), '', () => {
    customRow.hidden = layout.value !== '__custom';
  });
  const custom = h('input', { class: 'input', placeholder: 'us,de', spellcheck: false, maxLength: 256 });
  const customRow = row('Custom layouts', 'XKB layout names separated by commas, for example us,de.', custom);
  customRow.hidden = true;
  const variant = h('input', { class: 'input', placeholder: 'Default (or intl, dvorak…)', spellcheck: false, maxLength: 256 });
  const options = h('input', { class: 'input', placeholder: 'For example grp:alt_shift_toggle', spellcheck: false, maxLength: 256 });
  const rate = h('input', { class: 'input', type: 'number', required: true, min: 0, max: 100, step: 1 });
  const delay = h('input', { class: 'input', type: 'number', required: true, min: 100, max: 2000, step: 25 });
  const test = h('textarea', { class: 'input input-typing-test', rows: 2, placeholder: 'Type here to try your layout and repeat settings.', 'aria-label': 'Try keyboard settings', spellcheck: false });
  const profile = selectBox([
    { value: 'default', label: 'Device default' }, { value: 'flat', label: 'Flat — constant speed' },
    { value: 'adaptive', label: 'Adaptive — faster when you move faster' },
  ], 'default', () => {});
  const speed = h('input', { type: 'range', min: -1, max: 1, step: 0.05, value: 0 });
  const speedValue = h('output', { class: 'mono' }, '0.00');
  speed.addEventListener('input', () => { speedValue.textContent = Number(speed.value).toFixed(2); });
  const left = toggle(false, () => {});
  const natural = toggle(false, () => {});
  const checkbox = (control: HTMLElement) => control.querySelector('input')!;

  function fill(s: InputSettings): void {
    layout.value = s.keyboard_layout;
    if (layout.value !== s.keyboard_layout) layout.value = '__custom';
    custom.value = s.keyboard_layout;
    customRow.hidden = layout.value !== '__custom';
    variant.value = s.keyboard_variant;
    options.value = s.keyboard_options;
    rate.value = String(s.repeat_rate);
    delay.value = String(s.repeat_delay);
    profile.value = s.mouse_profile;
    speed.value = String(s.mouse_speed);
    speedValue.textContent = s.mouse_speed.toFixed(2);
    checkbox(left).checked = s.mouse_left_handed;
    checkbox(natural).checked = s.mouse_natural_scroll;
  }

  async function refreshDevices(): Promise<void> {
    const state = await bridge.call<InputState>('input.get');
    if (!alive) return;
    devices.replaceChildren(...(state.mice.length ? state.mice.map((d) => h('p', {},
      h('strong', {}, d.name), ': ', d.acceleration ? `${d.profile ?? 'device'} acceleration, speed ${d.speed.toFixed(2)}` : 'Absolute pointer; speed and acceleration are controlled by the device.')) :
      [h('p', {}, 'No mouse is connected. Saved mouse settings apply when one is connected.')]));
  }

  async function save(next: InputSettings): Promise<void> {
    if (busy) return;
    busy = true;
    form.disabled = true;
    try {
      const result = await bridge.call<{ prefs: Prefs }>('prefs.set', { prefs: { input: next } });
      if (!result.prefs.input) throw new Error('The compositor did not confirm the input settings.');
      if (!alive) return;
      current = { ...INPUT_DEFAULTS, ...result.prefs.input };
      fill(current);
      note.show('Input settings applied and saved.', 'ok');
      await refreshDevices();
    } catch (error) {
      if (alive) note.show(bridge.reason(error), 'error');
    } finally {
      busy = false;
      if (alive) form.disabled = false;
    }
  }

  const apply = h('button', { class: 'btn primary', type: 'button', onclick: () => {
    if (!rate.reportValidity() || !delay.reportValidity()) return;
    void save({ keyboard_layout: layout.value === '__custom' ? custom.value.trim() : layout.value,
      keyboard_variant: variant.value.trim(), keyboard_options: options.value.trim(),
      repeat_rate: Number(rate.value), repeat_delay: Number(delay.value), mouse_profile: profile.value,
      mouse_speed: Number(speed.value), mouse_left_handed: checkbox(left).checked,
      mouse_natural_scroll: checkbox(natural).checked });
  } }, 'Apply input settings');
  const reset = h('button', { class: 'btn', type: 'button', onclick: () => {
    fill(INPUT_DEFAULTS); note.show('Defaults selected. Apply to use them.');
  } }, 'Choose defaults');
  const undo = h('button', { class: 'btn', type: 'button', onclick: () => fill(current) }, 'Discard edits');
  form.append(
    card('Keyboard', row('Layout', 'Changes apply to this desktop session and are remembered for your next login.', layout), customRow,
      row('Repeat rate', 'Keys per second while held. Set to 0 to turn repeat off.', rate),
      row('Repeat delay', 'Milliseconds before a held key starts repeating.', delay),
      h('details', { class: 'input-advanced' }, h('summary', {}, 'Advanced layout options'),
        row('Variant', 'Optional XKB variant. For multiple layouts, use one entry per layout.', variant),
        row('Options', 'Compose keys and layout switching, such as compose:ralt or grp:alt_shift_toggle.', options)), test),
    card('Mouse', row('Acceleration', 'Flat keeps a constant response. Games using raw input keep their own sensitivity.', profile),
      row('Pointer speed', 'Slower to the left, faster to the right. 0 is the neutral speed.', h('div', { class: 'input-speed' }, speed, speedValue)),
      row('Left-handed buttons', 'Swap the primary and secondary mouse buttons on supported devices.', left),
      row('Natural scrolling', 'Reverse the mouse wheel direction on supported devices.', natural), devices),
    h('div', { class: 'card-actions input-settings-actions' }, reset, undo, h('span', { class: 'strip-gap' }), apply),
  );
  el.append(pageHeader('Keyboard & mouse', 'Keyboard layout, repeat rate and pointer settings.'), note.el, form);
  void bridge.call<InputState>('input.get').then(async (state) => {
    if (!alive) return;
    current = { ...state.settings };
    fill(current);
    form.disabled = false;
    await refreshDevices();
  }).catch((error) => { if (alive) note.show(`Input settings are unavailable: ${error}`, 'error'); });
  return () => { alive = false; };
}
