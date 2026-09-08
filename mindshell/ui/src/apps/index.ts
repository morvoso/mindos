// App windows: the same bundle rendered in a normal (decorated) window by
// `mindshell --app NAME`. The app name comes from the window id.

import { h } from '../dom';
import { glassLayer } from '../glass';
import { store } from '../state';
import { renderGameLibrary } from '../game-library';
import { renderSettings } from './settings';
import { renderGaming } from './gaming';
import { renderCompanion } from './companion';

export function renderApp(root: HTMLElement, name: string, arg: unknown): void {
  root.classList.add('app-window');
  root.dataset.app = name;
  const a = (arg && typeof arg === 'object' ? arg : {}) as { page?: string; arg?: string };
  switch (name) {
    case 'gaming':
      renderGaming(root, a.page, a.arg);
      break;
    case 'companion':
      renderCompanion(root, a.arg);
      break;
    case 'library':
      renderGameLibrary(root);
      break;
    case 'settings':
      renderSettings(root, a.page);
      break;
    default:
      root.appendChild(h('div', { class: 'app-empty' }, `There is no app called “${name}”.`));
  }
  const glass = glassLayer(root, {
    output: store.state.outputs[0]?.name || '',
    origin: () => ({ x: 0, y: 0 }),
  });
  window.addEventListener('pagehide', glass.dispose, { once: true });
}
