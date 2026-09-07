// App windows: the same bundle rendered in a normal (decorated) window by
// `mindshell --app NAME`. The app name comes from the window id.

import { h } from '../dom';
import { renderSettings } from './settings';

export function renderApp(root: HTMLElement, name: string, arg: unknown): void {
  root.classList.add('app-window');
  root.dataset.app = name;
  const a = (arg && typeof arg === 'object' ? arg : {}) as { page?: string; arg?: string };
  switch (name) {
    case 'settings':
      renderSettings(root, a.page);
      break;
    default:
      root.appendChild(h('div', { class: 'app-empty' }, `There is no app called “${name}”.`));
  }
}
