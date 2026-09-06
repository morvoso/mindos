import { h } from '../dom';
import { icon } from '../icons';
import { newWidget, panelById } from '../layout';
import { allWidgets } from '../widgets/registry';
import type { Container, DesktopWidgetEntry } from '../types';
import type { PopupContent, PopupCtx } from './shared';

interface Target {
  kind: Container;
  id?: string;
  output?: string;
}

export function widgetCatalogPopup(ctx: PopupCtx): PopupContent {
  const store = ctx.store;
  const target = (ctx.arg.target as Target | undefined) ?? { kind: 'desktop', output: ctx.output };
  let container: Container = target.kind;

  const tabs = h('div', { class: 'segs' });
  const grid = h('div', { class: 'cat-grid' });
  const hint = h('div', { class: 'pop-hint' });

  const add = (type: string, card: HTMLElement) => {
    const def = allWidgets().find((d) => d.type === type);
    if (!def) return;
    store.updateLayout((l) => {
      if (container === 'panel') {
        const p = panelById(l, target.id ?? '') ?? l.panels[0];
        if (!p) return;
        p.widgets.push(newWidget(type, def.defaults));
      } else {
        const out = store.output(target.output ?? ctx.output);
        const n = l.desktop.widgets.length;
        const size = def.defaultSize ?? { w: 280, h: 160 };
        const entry: DesktopWidgetEntry = {
          ...newWidget(type, def.defaults),
          output: target.output ?? ctx.output ?? '*',
          x: Math.min(64 + n * 32, (out?.width ?? 1920) - size.w - 32),
          y: Math.min(220 + n * 32, (out?.height ?? 1080) - size.h - 120),
          w: size.w,
          h: size.h,
        };
        l.desktop.widgets.push(entry);
      }
    });
    const btn = card.querySelector('.btn')!;
    btn.classList.add('ok');
    btn.replaceChildren(icon('check', 14), 'Added');
    setTimeout(() => {
      btn.classList.remove('ok');
      btn.replaceChildren(icon('plus', 14), 'Add');
    }, 1200);
  };

  const render = () => {
    tabs.replaceChildren(
      ...(['panel', 'desktop'] as Container[]).map((c) =>
        h('button', { class: `seg${c === container ? ' on' : ''}`, onclick: () => {
          container = c;
          render();
        } }, c === 'panel' ? 'PANEL' : 'DESKTOP'),
      ),
    );
    const canPanel = container === 'panel' && (target.kind !== 'panel' || !panelById(store.state.layout, target.id ?? ''));
    hint.textContent =
      container === 'panel'
        ? canPanel
          ? 'Widgets are added to the first panel'
          : `Widgets are added to the ${target.id} panel`
        : 'Widgets are placed on the desktop; drag to arrange';
    grid.replaceChildren(
      ...allWidgets(container).map((d) => {
        const card = h(
          'div',
          { class: 'cat-card' },
          h('span', { class: 'cat-ic' }, icon(d.icon, 22)),
          h('div', { class: 'cat-text' }, h('div', { class: 'cat-name' }, d.name), h('div', { class: 'cat-desc' }, d.description)),
          h('button', { class: 'btn small accent', onclick: () => add(d.type, card) }, icon('plus', 14), 'Add'),
        );
        return card;
      }),
    );
    ctx.relayout();
  };
  render();
  const el = h(
    'div',
    { class: 'pop-body catalog' },
    h('div', { class: 'pop-head' }, h('span', { class: 'pop-title' }, 'ADD WIDGET'), tabs, h('span', { class: 'lch-gap' }), h('button', { class: 'tool', title: 'Close', onclick: () => ctx.close() }, icon('x', 14))),
    grid,
    hint,
  );
  return { el, w: 640 };
}
