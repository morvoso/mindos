// Settings › Displays: resolution, refresh rate and scale (Basic), plus
// enable/primary/orientation/VRR and the arrangement (Advanced). Changes that
// can leave a screen dark ask to be kept within 15 seconds and revert otherwise.

import * as bridge from '../bridge';
import { clamp, h } from '../dom';
import { icon } from '../icons';
import { store } from '../state';
import type { OutputChange, WmMode, WmOutput } from '../types';
import { card, dialog, notice, pageHeader, pill, progress, row, selectBox, toggle } from './shared';

const TRANSFORMS: { value: string; label: string }[] = [
  { value: 'normal', label: 'Landscape' },
  { value: '90', label: 'Portrait (rotated right)' },
  { value: '270', label: 'Portrait (rotated left)' },
  { value: '180', label: 'Landscape (upside down)' },
  { value: 'flipped', label: 'Landscape, mirrored' },
  { value: 'flipped-90', label: 'Portrait right, mirrored' },
  { value: 'flipped-270', label: 'Portrait left, mirrored' },
  { value: 'flipped-180', label: 'Upside down, mirrored' },
];

const SCALES = [100, 125, 150, 175, 200, 250, 300];
const KEEP_SECONDS = 15;

interface Pending {
  res?: string;
  refresh?: number;
  scale?: number;
  transform?: string;
}

const hz = (mhz: number) => {
  const v = mhz / 1000;
  return `${Number.isInteger(v) ? v : v.toFixed(2).replace(/\.?0+$/, '')} Hz`;
};
const resKey = (m: { width: number; height: number }) => `${m.width}x${m.height}`;
const inches = (o: WmOutput) => (o.mm_width && o.mm_height ? `${(Math.hypot(o.mm_width, o.mm_height) / 25.4).toFixed(1)}″` : '');

export function displaysPage(el: HTMLElement, root: HTMLElement): () => void {
  const note = notice();
  let outputs: WmOutput[] = [];
  let advanced = false;
  const pending = new Map<string, Pending>();
  const cards = h('div', { class: 'stack' });
  const canvasCard = card('Arrangement', h('p', { class: 'card-help' }, 'Drag a display to move it. Displays align at the top edge when placed close together.'), h('div', { class: 'arrange' }));
  const arrange = canvasCard.querySelector('.arrange') as HTMLElement;

  const modeBtns = [
    h('button', { class: 'seg', onclick: () => setAdvanced(false) }, 'Basic'),
    h('button', { class: 'seg', onclick: () => setAdvanced(true) }, 'Advanced'),
  ];
  const setAdvanced = (v: boolean) => {
    advanced = v;
    modeBtns[0].classList.toggle('on', !v);
    modeBtns[1].classList.toggle('on', v);
    canvasCard.hidden = !v;
    render();
  };

  el.append(pageHeader('Displays', 'Resolution, refresh rate and scale. Advanced adds orientation, variable refresh rate and the arrangement.', h('span', { class: 'segs' }, ...modeBtns)), note.el, cards, canvasCard);

  const fail = (e: unknown) => note.show(`Display: ${e instanceof Error ? e.message : String(e)}`, 'error');

  const fetch = () =>
    bridge
      .call<{ outputs: WmOutput[] }>('wm.outputs')
      .then((r) => {
        outputs = r?.outputs ?? [];
        render();
      })
      .catch(fail);

  const apply = (name: string, change: OutputChange) =>
    bridge.call<{ outputs: WmOutput[] }>('wm.setOutput', { name, ...change }).then((r) => {
      if (r?.outputs) outputs = r.outputs;
      render();
    });

  /** Apply a change that could leave the screen unusable, then ask to keep it. */
  const applyWithConfirm = async (o: WmOutput, change: OutputChange) => {
    const before: OutputChange = {
      mode: currentMode(o) ? { width: currentMode(o)!.width, height: currentMode(o)!.height, refresh: currentMode(o)!.refresh } : undefined,
      scale: o.scale,
      transform: o.transform,
    };
    try {
      await apply(o.name, change);
    } catch (e) {
      fail(e);
      return;
    }
    pending.delete(o.name);
    render();
    let left = KEEP_SECONDS;
    const text = h('div', { class: 'row-help' }, `Reverting in ${left} s unless confirmed.`);
    const bar = progress(1, 'accent');
    let closeFn: (() => void) | undefined;
    const revert = () => {
      clearInterval(t);
      closeFn?.();
      apply(o.name, before).catch(fail);
      note.show(`Reverted ${o.name}.`, 'info');
    };
    const keep = () => {
      clearInterval(t);
      closeFn?.();
      note.show(`${o.name}: settings kept.`, 'ok');
    };
    const t = setInterval(() => {
      left -= 1;
      text.textContent = `Reverting in ${left} s if you do nothing.`;
      (bar.firstElementChild as HTMLElement).style.width = `${(left / KEEP_SECONDS) * 100}%`;
      if (left <= 0) revert();
    }, 1000);
    closeFn = dialog(root, `Keep these settings for ${o.name}?`, h('div', { class: 'stack' }, text, bar), [
      h('button', { class: 'btn', onclick: revert }, 'Revert'),
      h('button', { class: 'btn primary', onclick: keep }, icon('check', 14), 'Keep'),
    ]);
  };

  const currentMode = (o: WmOutput): WmMode | undefined => o.modes.find((m) => m.current) ?? o.modes.find((m) => m.width === o.width && m.height === o.height);

  const resolutions = (o: WmOutput) => {
    const seen = new Map<string, WmMode>();
    for (const m of [...o.modes].sort((a, b) => b.width * b.height - a.width * a.height || b.refresh - a.refresh)) if (!seen.has(resKey(m))) seen.set(resKey(m), m);
    return [...seen.values()];
  };

  const outputCard = (o: WmOutput) => {
    const p = pending.get(o.name) ?? {};
    const cur = currentMode(o);
    const res = p.res ?? (cur ? resKey(cur) : '');
    const rates = o.modes.filter((m) => resKey(m) === res).map((m) => m.refresh).sort((a, b) => b - a);
    const refresh = p.refresh !== undefined && rates.includes(p.refresh) ? p.refresh : cur && resKey(cur) === res ? cur.refresh : rates[0];
    const scale = p.scale ?? Math.round(o.scale * 100);
    const transform = p.transform ?? o.transform;
    const set = (patch: Pending) => {
      pending.set(o.name, { ...pending.get(o.name), ...patch });
      render();
    };
    const dirty = (cur && (res !== resKey(cur) || refresh !== cur.refresh)) || scale !== Math.round(o.scale * 100) || transform !== o.transform;

    const resSel = selectBox(resolutions(o).map((m) => ({ value: resKey(m), label: `${m.width} × ${m.height}${m.preferred ? '  (native)' : ''}` })), res, (v) => set({ res: v, refresh: undefined }));
    const rateSel = selectBox(rates.map((r) => ({ value: r, label: hz(r) + (o.modes.some((m) => resKey(m) === res && m.refresh === r && m.preferred) ? '  (native)' : '') })), refresh, (v) => set({ refresh: v }));
    const scaleSel = selectBox(SCALES.map((s) => ({ value: s, label: `${s}%` })), SCALES.includes(scale) ? scale : 100, (v) => set({ scale: v }));
    const applyBtn = h('button', { class: 'btn primary', disabled: !dirty, onclick: () => {
      const mode = o.modes.find((m) => resKey(m) === res && m.refresh === refresh);
      const change: OutputChange = {};
      if (mode && (!cur || mode.width !== cur.width || mode.height !== cur.height || mode.refresh !== cur.refresh)) change.mode = { width: mode.width, height: mode.height, refresh: mode.refresh };
      if (scale !== Math.round(o.scale * 100)) change.scale = scale / 100;
      if (transform !== o.transform) change.transform = transform;
      void applyWithConfirm(o, change);
    } }, icon('check', 14), 'Apply');
    const badges = h('span', { class: 'pills' });
    if (o.primary) badges.appendChild(pill('Primary', 'accent'));
    if (!o.enabled) badges.appendChild(pill('Off', 'warn'));
    if (o.vrr) badges.appendChild(pill('VRR', 'ok'));
    const head = h('div', { class: 'out-head' }, h('span', { class: 'out-ic' }, icon('monitor', 20)), h('div', { class: 'out-text' }, h('div', { class: 'out-name' }, o.name, badges), h('div', { class: 'row-help' }, [o.make, o.model, inches(o)].filter(Boolean).join(' · ') || 'Unknown display')), h('span', { class: 'strip-gap' }), h('span', { class: 'mono out-now' }, cur ? `${cur.width}×${cur.height} @ ${hz(cur.refresh)}` : ''));

    const c = card(null, head);
    if (!o.enabled && !advanced) {
      c.appendChild(row('This display is disabled', 'Enable it under Advanced.', null));
      return c;
    }
    if (o.enabled) {
      c.append(
        row('Resolution', null, resSel),
        row('Refresh rate', 'Higher rates are smoother. Games use this rate unless variable refresh rate is enabled.', rateSel),
        row('Scale', 'Enlarges the interface on high-resolution displays.', scaleSel),
      );
    }
    if (advanced) {
      const onlyOne = outputs.filter((x) => x.enabled).length <= 1 && o.enabled;
      c.append(
        row('Enabled', onlyOne ? 'The last display cannot be turned off.' : null, toggle(o.enabled, (v) => apply(o.name, { enabled: v }).catch(fail), onlyOne)),
        row('Primary display', 'The top bar and the dock are shown on this display.', toggle(o.primary, (v) => apply(o.name, { primary: v }).catch(fail), o.primary || !o.enabled)),
      );
      if (o.enabled) {
        c.append(
          row('Orientation', null, selectBox(TRANSFORMS, transform, (v) => set({ transform: v }))),
          row('Variable refresh rate', o.vrr_supported ? 'FreeSync / G-Sync: the display refresh rate follows the game’s frame rate.' : 'Not supported by this display or driver.', toggle(o.vrr, (v) => apply(o.name, { vrr: v }).catch(fail), !o.vrr_supported)),
          row('Position', 'Position of this display in the arrangement, in pixels.', positionFields(o)),
        );
      }
    }
    if (o.enabled) c.appendChild(h('div', { class: 'card-actions' }, dirty ? h('span', { class: 'row-help' }, 'The change must be confirmed within 15 seconds.') : null, h('span', { class: 'strip-gap' }), applyBtn));
    return c;
  };

  const positionFields = (o: WmOutput) => {
    const x = h('input', { type: 'number', value: o.x, step: 1 }) as HTMLInputElement;
    const y = h('input', { type: 'number', value: o.y, step: 1 }) as HTMLInputElement;
    const go = h('button', { class: 'btn small', onclick: () => apply(o.name, { position: [Number(x.value) || 0, Number(y.value) || 0] }).catch(fail) }, 'Move');
    return h('span', { class: 'inline-form compact' }, h('span', { class: 'mono dim' }, 'X'), x, h('span', { class: 'mono dim' }, 'Y'), y, go);
  };

  // ----- arrangement canvas ---------------------------------------------------

  const renderArrange = () => {
    arrange.replaceChildren();
    const on = outputs.filter((o) => o.enabled);
    if (!on.length) return;
    const minX = Math.min(...on.map((o) => o.x));
    const minY = Math.min(...on.map((o) => o.y));
    const maxX = Math.max(...on.map((o) => o.x + o.width));
    const maxY = Math.max(...on.map((o) => o.y + o.height));
    const W = arrange.clientWidth || 640;
    const H = 260;
    const s = Math.min((W - 40) / Math.max(1, maxX - minX), (H - 40) / Math.max(1, maxY - minY), 0.25);
    const ox = (W - (maxX - minX) * s) / 2;
    const oy = (H - (maxY - minY) * s) / 2;
    arrange.style.height = `${H}px`;
    for (const o of on) {
      const box = h('div', { class: `arr-out${o.primary ? ' primary' : ''}`, style: { left: `${ox + (o.x - minX) * s}px`, top: `${oy + (o.y - minY) * s}px`, width: `${o.width * s}px`, height: `${o.height * s}px` } }, h('span', { class: 'arr-name' }, o.name), h('span', { class: 'arr-sub mono' }, `${o.width}×${o.height}`));
      box.addEventListener('pointerdown', (e) => {
        if (e.button !== 0) return;
        e.preventDefault();
        box.setPointerCapture(e.pointerId);
        const start = { px: e.clientX, py: e.clientY, x: o.x, y: o.y };
        let cur = { x: o.x, y: o.y };
        let moved = false;
        const move = (ev: PointerEvent) => {
          const dx = (ev.clientX - start.px) / s;
          const dy = (ev.clientY - start.py) / s;
          if (!moved && Math.hypot(dx, dy) < 4) return;
          moved = true;
          cur = { x: Math.round(start.x + dx), y: Math.round(start.y + dy) };
          // Snap to the edges of the other displays.
          for (const p of on) {
            if (p === o) continue;
            const snapTo = (v: number, t: number) => (Math.abs(v - t) < 24 / s ? t : v);
            cur.x = snapTo(cur.x, p.x + p.width);
            cur.x = snapTo(cur.x + o.width, p.x) - o.width;
            cur.y = snapTo(cur.y, p.y);
            cur.y = snapTo(cur.y, p.y + p.height);
            cur.y = snapTo(cur.y + o.height, p.y + p.height) - o.height;
          }
          box.style.left = `${ox + (cur.x - minX) * s}px`;
          box.style.top = `${oy + (cur.y - minY) * s}px`;
          box.classList.add('dragging');
        };
        const up = () => {
          box.removeEventListener('pointermove', move);
          box.removeEventListener('pointerup', up);
          box.classList.remove('dragging');
          if (!moved) return;
          apply(o.name, { position: [clamp(cur.x, -32768, 32767), clamp(cur.y, -32768, 32767)] }).catch((err) => {
            fail(err);
            render();
          });
        };
        box.addEventListener('pointermove', move);
        box.addEventListener('pointerup', up);
      });
      arrange.appendChild(box);
    }
  };

  const render = () => {
    cards.replaceChildren();
    if (!outputs.length) cards.appendChild(card(null, h('div', { class: 'row-help' }, 'No displays reported yet.')));
    for (const o of outputs) cards.appendChild(outputCard(o));
    if (advanced) renderArrange();
  };

  setAdvanced(false);
  void fetch();
  store.bind(el, 'outputs', () => void fetch());
  const ro = typeof ResizeObserver === 'function' ? new ResizeObserver(() => advanced && renderArrange()) : undefined;
  ro?.observe(arrange);
  return () => ro?.disconnect();
}
