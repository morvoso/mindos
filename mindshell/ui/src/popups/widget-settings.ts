import { h } from '../dom';
import { icon } from '../icons';
import { panelById } from '../layout';
import { getWidget, mergedConfig, type SettingSpec } from '../widgets/registry';
import type { Config, WidgetEntry } from '../types';
import type { PopupContent, PopupCtx } from './shared';

interface Target {
  kind: 'panel' | 'desktop';
  id?: string;
  widget: string;
}

/** Derive a settings form from the defaults when a widget declares none. */
function inferSpecs(defaults: Config): Record<string, SettingSpec> {
  const specs: Record<string, SettingSpec> = {};
  for (const [k, v] of Object.entries(defaults)) {
    const label = k.replace(/([A-Z])/g, ' $1').replace(/^./, (c) => c.toUpperCase());
    if (typeof v === 'boolean') specs[k] = { label, type: 'boolean' };
    else if (typeof v === 'number') specs[k] = { label, type: 'number' };
    else if (Array.isArray(v)) specs[k] = { label, type: 'list' };
    else if (typeof v === 'string') specs[k] = { label, type: v.length > 60 ? 'text' : 'string' };
  }
  return specs;
}

export function widgetSettingsPopup(ctx: PopupCtx): PopupContent {
  const store = ctx.store;
  const target = ctx.arg.target as Target | undefined;
  const find = (): WidgetEntry | undefined => {
    if (!target) return undefined;
    if (target.kind === 'panel') return panelById(store.state.layout, target.id ?? '')?.widgets.find((w) => w.id === target.widget);
    return store.state.layout.desktop.widgets.find((w) => w.id === target.widget);
  };
  const entry = find();
  const def = entry ? getWidget(entry.type) : undefined;
  const el = h('div', { class: 'pop-body settings' });
  if (!entry) {
    el.append(h('div', { class: 'pop-title' }, 'WIDGET SETTINGS'), h('div', { class: 'pop-hint' }, 'This widget no longer exists.'));
    return { el, w: 360 };
  }
  const specs = def?.settings ?? inferSpecs(def?.defaults ?? {});
  const values: Config = mergedConfig(def, entry.config);
  const form = h('div', { class: 'form' });
  const controls = new Map<string, () => unknown>();

  for (const [key, spec] of Object.entries(specs)) {
    const v = values[key];
    let ctl: HTMLElement;
    let read: () => unknown;
    switch (spec.type) {
      case 'boolean': {
        const input = h('input', { type: 'checkbox', checked: !!v }) as HTMLInputElement;
        ctl = h('label', { class: 'switch' }, input, h('i'));
        read = () => input.checked;
        break;
      }
      case 'number': {
        const input = h('input', { type: 'number', value: String(v ?? ''), min: spec.min, max: spec.max, step: spec.step ?? 1 }) as HTMLInputElement;
        ctl = input;
        read = () => (input.value === '' ? undefined : Number(input.value));
        break;
      }
      case 'enum': {
        const select = h('select') as HTMLSelectElement;
        for (const o of spec.options ?? []) select.appendChild(h('option', { value: String(o.value), selected: String(o.value) === String(v) }, o.label));
        ctl = select;
        read = () => {
          const o = (spec.options ?? []).find((x) => String(x.value) === select.value);
          return o ? o.value : select.value;
        };
        break;
      }
      case 'text': {
        const ta = h('textarea', { rows: 4 }) as HTMLTextAreaElement;
        ta.value = String(v ?? '');
        ctl = ta;
        read = () => ta.value;
        break;
      }
      case 'list': {
        const ta = h('textarea', { rows: 4, placeholder: 'one per line' }) as HTMLTextAreaElement;
        ta.value = Array.isArray(v) ? (v as unknown[]).map(String).join('\n') : '';
        ctl = ta;
        read = () => ta.value.split('\n').map((s) => s.trim()).filter(Boolean);
        break;
      }
      default: {
        const input = h('input', { type: 'text', value: String(v ?? '') }) as HTMLInputElement;
        ctl = input;
        read = () => input.value;
      }
    }
    controls.set(key, read);
    const row = h('label', { class: `frow frow-${spec.type}` }, h('span', { class: 'frow-label' }, spec.label, spec.help ? h('small', {}, spec.help) : null), ctl);
    form.appendChild(row);
  }
  if (!controls.size) form.appendChild(h('div', { class: 'pop-hint' }, 'This widget has no settings.'));

  const save = () => {
    const next: Config = {};
    for (const [k, read] of controls) {
      const v = read();
      if (v !== undefined) next[k] = v;
    }
    void store.updateLayout((l) => {
      const e = target!.kind === 'panel' ? panelById(l, target!.id ?? '')?.widgets.find((w) => w.id === target!.widget) : l.desktop.widgets.find((w) => w.id === target!.widget);
      if (e) e.config = { ...e.config, ...next };
    });
    ctx.close();
  };
  const reset = () => {
    void store.updateLayout((l) => {
      const e = target!.kind === 'panel' ? panelById(l, target!.id ?? '')?.widgets.find((w) => w.id === target!.widget) : l.desktop.widgets.find((w) => w.id === target!.widget);
      if (e) e.config = {};
    });
    ctx.close();
  };
  el.append(
    h('div', { class: 'pop-head' }, h('span', { class: 'set-ic' }, icon(def?.icon ?? 'box', 18)), h('span', { class: 'pop-title' }, (def?.name ?? entry.type).toUpperCase()), h('span', { class: 'lch-gap' }), h('button', { class: 'tool', title: 'Close', onclick: () => ctx.close() }, icon('x', 14))),
    form,
    h('div', { class: 'pop-actions' }, h('button', { class: 'btn', onclick: reset }, icon('refresh', 14), 'Defaults'), h('span', { class: 'lch-gap' }), h('button', { class: 'btn', onclick: () => ctx.close() }, 'Cancel'), h('button', { class: 'btn primary', onclick: save }, icon('check', 14), 'Save')),
  );
  el.addEventListener('keydown', (e) => {
    if (e.key === 'Enter' && (e.target as HTMLElement).tagName !== 'TEXTAREA') save();
  });
  return { el, w: 400, focus: () => (form.querySelector('input, select, textarea') as HTMLElement | null)?.focus() };
}
