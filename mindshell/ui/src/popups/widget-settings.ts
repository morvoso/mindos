// The settings form for one widget. Built from the widget's `settings`
// specs (or inferred from its defaults); every change applies to the widget
// at once and is saved, so the panel is the live preview. "Defaults" clears
// the widget's config.

import { debounce, h } from '../dom';
import { icon } from '../icons';
import { panelById } from '../layout';
import { getWidget, mergedConfig, type SettingSpec } from '../widgets/registry';
import type { Config, Layout, WidgetEntry } from '../types';
import type { PopupContent, PopupCtx } from './shared';

export interface SettingsTarget {
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

function findEntry(l: Layout, t: SettingsTarget): WidgetEntry | undefined {
  if (t.kind === 'panel') return panelById(l, t.id ?? '')?.widgets.find((w) => w.id === t.widget);
  return l.desktop.widgets.find((w) => w.id === t.widget);
}

interface Control {
  row: HTMLElement;
  read(): unknown;
  set(v: unknown): void;
}

function control(key: string, spec: SettingSpec, value: unknown, onChange: (immediate: boolean) => void): Control {
  const label = h('span', { class: 'frow-label' }, spec.label, spec.help ? h('small', {}, spec.help) : null);
  switch (spec.type) {
    case 'boolean': {
      const input = h('input', { type: 'checkbox', checked: !!value }) as HTMLInputElement;
      input.addEventListener('change', () => onChange(true));
      return {
        row: h('label', { class: 'frow frow-boolean', dataset: { key } }, label, h('span', { class: 'switch' }, input, h('i'))),
        read: () => input.checked,
        set: (v) => (input.checked = !!v),
      };
    }
    case 'number': {
      const num = h('input', { type: 'number', value: String(value ?? ''), min: spec.min, max: spec.max, step: spec.step ?? 1 }) as HTMLInputElement;
      const withSlider = spec.slider !== false && spec.min !== undefined && spec.max !== undefined;
      const range = withSlider ? (h('input', { type: 'range', value: String(value ?? spec.min), min: spec.min, max: spec.max, step: spec.step ?? 1 }) as HTMLInputElement) : null;
      const unit = spec.unit ? h('span', { class: 'frow-unit' }, spec.unit) : null;
      num.addEventListener('input', () => {
        if (range && num.value !== '') range.value = num.value;
        onChange(false);
      });
      num.addEventListener('change', () => onChange(true));
      range?.addEventListener('input', () => {
        num.value = range.value;
        onChange(false);
      });
      range?.addEventListener('change', () => onChange(true));
      const ctl = h('span', { class: `frow-num${range ? ' with-slider' : ''}` }, range, num, unit);
      return {
        row: h('label', { class: `frow frow-number${range ? ' frow-slider' : ''}`, dataset: { key } }, label, ctl),
        read: () => (num.value === '' ? undefined : Number(num.value)),
        set: (v) => {
          num.value = v === undefined || v === null ? '' : String(v);
          if (range) range.value = num.value || String(spec.min);
        },
      };
    }
    case 'enum': {
      const options = spec.options ?? [];
      if (spec.segmented) {
        let cur = String(value);
        const segs = h('span', { class: 'segs frow-segs' });
        const sync = () => {
          for (const b of Array.from(segs.children) as HTMLElement[]) b.classList.toggle('on', b.dataset.value === cur);
        };
        for (const o of options) {
          const b = h('button', { type: 'button', class: 'seg', dataset: { value: String(o.value) } }, o.label);
          b.addEventListener('click', () => {
            cur = String(o.value);
            sync();
            onChange(true);
          });
          segs.appendChild(b);
        }
        sync();
        return {
          row: h('div', { class: 'frow frow-enum', dataset: { key } }, label, segs),
          read: () => options.find((o) => String(o.value) === cur)?.value ?? cur,
          set: (v) => {
            cur = String(v);
            sync();
          },
        };
      }
      const select = h('select') as HTMLSelectElement;
      for (const o of options) select.appendChild(h('option', { value: String(o.value), selected: String(o.value) === String(value) }, o.label));
      select.addEventListener('change', () => onChange(true));
      return {
        row: h('label', { class: 'frow frow-enum', dataset: { key } }, label, select),
        read: () => options.find((o) => String(o.value) === select.value)?.value ?? select.value,
        set: (v) => (select.value = String(v)),
      };
    }
    case 'text':
    case 'list': {
      const ta = h('textarea', { rows: 4, placeholder: spec.placeholder ?? (spec.type === 'list' ? 'one per line' : undefined) }) as HTMLTextAreaElement;
      const show = (v: unknown) => (spec.type === 'list' ? (Array.isArray(v) ? (v as unknown[]).map(String).join('\n') : '') : String(v ?? ''));
      ta.value = show(value);
      ta.addEventListener('input', () => onChange(false));
      ta.addEventListener('change', () => onChange(true));
      return {
        row: h('label', { class: `frow frow-${spec.type}`, dataset: { key } }, label, ta),
        read: () => (spec.type === 'list' ? ta.value.split('\n').map((s) => s.trim()).filter(Boolean) : ta.value),
        set: (v) => (ta.value = show(v)),
      };
    }
    default: {
      const input = h('input', { type: 'text', value: String(value ?? ''), placeholder: spec.placeholder }) as HTMLInputElement;
      input.addEventListener('input', () => onChange(false));
      input.addEventListener('change', () => onChange(true));
      return {
        row: h('label', { class: 'frow frow-string', dataset: { key } }, label, input),
        read: () => input.value,
        set: (v) => (input.value = String(v ?? '')),
      };
    }
  }
}

export function widgetSettingsPopup(ctx: PopupCtx): PopupContent {
  const store = ctx.store;
  const target = ctx.arg.target as SettingsTarget | undefined;
  const entry = target ? findEntry(store.state.layout, target) : undefined;
  const def = entry ? getWidget(entry.type) : undefined;
  const el = h('div', { class: 'pop-body settings' });
  if (!entry || !target) {
    el.append(h('div', { class: 'pop-title' }, 'WIDGET SETTINGS'), h('div', { class: 'pop-hint' }, 'This widget no longer exists.'));
    return { el, w: 360 };
  }
  const specs = def?.settings ?? inferSpecs(def?.defaults ?? {});
  const form = h('div', { class: 'form' });
  const controls = new Map<string, Control>();

  /** The config as the form shows it (defaults merged in). */
  const current = (): Config => {
    const next: Config = mergedConfig(def, entry.config);
    for (const [k, c] of controls) {
      const v = c.read();
      if (v !== undefined) next[k] = v;
    }
    return next;
  };

  const persist = (cfg: Config) =>
    store.updateLayout((l) => {
      const e = findEntry(l, target);
      if (e) e.config = { ...e.config, ...cfg };
    });
  const persistSoon = debounce(persist, 350);

  const syncVisibility = (cfg: Config) => {
    let changed = false;
    for (const [k, c] of controls) {
      const hide = !!specs[k].when && !specs[k].when!(cfg);
      if (c.row.hidden !== hide) changed = true;
      c.row.hidden = hide;
    }
    // Rows came or went: keep the popup snug against its anchor.
    if (changed) ctx.relayout();
  };

  const onChange = (immediate: boolean) => {
    const cfg = current();
    syncVisibility(cfg);
    if (immediate) void persist(cfg);
    else persistSoon(cfg);
  };

  const values = mergedConfig(def, entry.config);
  for (const [key, spec] of Object.entries(specs)) {
    const c = control(key, spec, values[key], onChange);
    controls.set(key, c);
    form.appendChild(c.row);
  }
  syncVisibility(values);
  if (!controls.size) form.appendChild(h('div', { class: 'pop-hint' }, 'This widget has nothing to set up.'));

  // Another window may change the layout (a second settings popup, edit
  // mode): keep the form in step while the user is not typing in it.
  store.bind(el, 'layout', () => {
    const e = findEntry(store.state.layout, target);
    if (!e) return ctx.close();
    const merged = mergedConfig(def, e.config);
    for (const [k, c] of controls) {
      if (c.row.contains(document.activeElement)) continue;
      if (JSON.stringify(c.read()) !== JSON.stringify(merged[k])) c.set(merged[k]);
    }
    syncVisibility(merged);
  });

  const reset = () => {
    void store.updateLayout((l) => {
      const e = findEntry(l, target);
      if (e) e.config = {};
    });
    const d = def?.defaults ?? {};
    for (const [k, c] of controls) c.set(d[k]);
    syncVisibility({ ...d });
  };
  const remove = () => {
    void store.updateLayout((l) => {
      if (target.kind === 'panel') {
        const p = panelById(l, target.id ?? '');
        if (p) p.widgets = p.widgets.filter((w) => w.id !== target.widget);
      } else l.desktop.widgets = l.desktop.widgets.filter((w) => w.id !== target.widget);
    });
    ctx.close();
  };
  const removeBtn = h('button', { class: 'btn danger', title: 'Remove this widget' }, icon('trash', 14), 'Remove');
  const removeLabel = removeBtn.lastChild as Text;
  let armed = false;
  removeBtn.addEventListener('click', () => {
    if (!armed) {
      armed = true;
      removeBtn.classList.add('armed');
      removeLabel.textContent = 'Confirm';
      setTimeout(() => {
        armed = false;
        removeBtn.classList.remove('armed');
        removeLabel.textContent = 'Remove';
      }, 3000);
      return;
    }
    remove();
  });

  el.append(h('div', { class: 'pop-head' }, h('span', { class: 'set-ic' }, icon(def?.icon ?? 'box', 18)), h('span', { class: 'pop-title' }, (def?.name ?? entry.type).toUpperCase()), h('span', { class: 'lch-gap' }), h('button', { class: 'tool', title: 'Close', onclick: () => ctx.close() }, icon('x', 14))));
  if (def?.description) el.appendChild(h('div', { class: 'set-desc' }, def.description));
  el.append(
    form,
    h('div', { class: 'pop-hint' }, 'Changes apply straight away.'),
    h('div', { class: 'pop-actions' }, h('button', { class: 'btn', onclick: reset }, icon('refresh', 14), 'Defaults'), removeBtn, h('span', { class: 'lch-gap' }), h('button', { class: 'btn primary', onclick: () => ctx.close() }, icon('check', 14), 'Done')),
  );
  el.addEventListener('keydown', (e) => {
    if (e.key === 'Enter' && (e.target as HTMLElement).tagName !== 'TEXTAREA') {
      onChange(true);
      ctx.close();
    }
  });
  return { el, w: 420, focus: () => (form.querySelector('input:not([type=range]), select, textarea, button') as HTMLElement | null)?.focus() };
}
