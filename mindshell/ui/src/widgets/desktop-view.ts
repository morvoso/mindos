// Which of the two views the primary screen is in: the home screen, or the
// windows in front of it. The home screen fades away when a window opens and
// comes back when the last one closes, so something has to say which of the
// two the screen is showing -- and the panels are the one thing on screen in
// both, which is why the answer lives here rather than in either view.

import * as bridge from '../bridge';
import { h } from '../dom';
import { registerWidget } from './registry';
import { panelItem } from './common';
import { isMainOutput } from '../workspace';

registerWidget({
  type: 'desktop-view',
  name: 'Desktop view',
  description: 'Shows whether the home screen or the open windows have the screen. Click to switch between them (Super+D).',
  icon: 'desktop',
  containers: ['panel'],
  defaults: { label: false },
  settings: { label: { label: 'Show the view name', type: 'boolean', help: 'Home or Windows next to the mark' } },
  create(ctx) {
    const el = panelItem(ctx, 'w-desktop-view');
    const mark = h('span', { class: 'w-view-mark' });
    const label = h('span', { class: 'w-label' });
    el.append(mark, label);
    let cfg = ctx.config;
    const render = () => {
      const home = ctx.store.state.desktopHome;
      // With nothing open the home screen is the ground rather than a page
      // over the windows, and there is nothing for the switch to switch to.
      const windows = ctx.store.state.windows.some(w => !w.minimized && isMainOutput(w.output ?? ''));
      el.dataset.view = home ? 'home' : 'windows';
      el.classList.toggle('is-only-view', home && !windows);
      el.setAttribute('aria-pressed', String(home));
      label.textContent = home ? 'Home' : 'Windows';
      label.hidden = !cfg.label || !!ctx.panel?.vertical;
      el.title = !home
        ? 'The windows have the screen · Super+D or click for the home screen'
        : windows
          ? 'The home screen is over the windows · Super+D or click to go back to them'
          : 'The home screen, with nothing open in front of it';
    };
    render();
    ctx.store.bind(el, 'desktopHome', render);
    ctx.store.bind(el, 'windows', render);
    el.addEventListener('click', () => bridge.send('desktop.toggle'));
    return {
      el,
      update(c) {
        cfg = c;
        render();
      },
    };
  },
});
