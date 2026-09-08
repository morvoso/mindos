// Native preferences live in layout.json: the host watches it across shell
// processes. Browser previews use localStorage; greeter choices are temporary.
import { icon } from './icons';
import { h } from './dom';
import { store } from './state';
import { frostedWallpaper } from './static-wallpaper';

export type Appearance = { theme: 'dark' | 'light' };
const KEY = 'mindos.appearance.v1';
const listeners = new Set<() => void>();
const native = () => location.protocol === 'mindos:' && document.documentElement.dataset.kind !== 'greeter';
export const appearance: Appearance = { theme: 'dark' };

function apply(): void {
  document.documentElement.dataset.theme = appearance.theme;
  document.documentElement.style.setProperty('--frosted-wallpaper', frostedWallpaper(appearance.theme === 'light'));
  document.documentElement.classList.toggle('software-rendered', store.state.outputs.some(o => o.software_rendering === true)
    || (location.protocol === 'mindos:' && !store.state.outputs.length));
  listeners.forEach((fn) => fn());
}

function read(): void {
  try {
    const value = JSON.parse(localStorage.getItem(KEY) || '{}');
    appearance.theme = value.theme === 'light' ? 'light' : 'dark';
  } catch { /* Restricted or damaged storage uses the defaults. */ }
  apply();
}

export function initAppearance(): void {
  read();
  const fromLayout = () => {
    if (!native()) return;
    const saved = store.state.layout.desktop.appearance;
    appearance.theme = saved?.theme === 'light' ? 'light' : 'dark';
    apply();
  };
  fromLayout();
  store.on('layout', fromLayout);
  store.on('outputs', apply);
  window.addEventListener('storage', (e) => { if (!native() && (e.key === KEY || e.key === null)) read(); });
}

export function setAppearance(patch: Partial<Appearance>): void {
  Object.assign(appearance, patch);
  try { localStorage.setItem(KEY, JSON.stringify(appearance)); } catch { /* Still works for this session. */ }
  apply();
  if (native()) void store.updateLayout((layout) => { layout.desktop.appearance = { ...appearance }; });
}

export function onAppearance(fn: () => void): () => void {
  listeners.add(fn);
  return () => { listeners.delete(fn); };
}

export function appearanceControls(): { el: HTMLElement; destroy: () => void } {
  const theme = h('button', { class: 'btn', 'aria-label': 'Use light theme' });
  const render = () => {
    theme.replaceChildren(icon(appearance.theme === 'dark' ? 'sun' : 'moon', 17));
    theme.title = `Use ${appearance.theme === 'dark' ? 'light' : 'dark'} theme`;
    theme.setAttribute('aria-label', `Use ${appearance.theme === 'dark' ? 'light' : 'dark'} theme`);
  };
  theme.onclick = () => setAppearance({ theme: appearance.theme === 'dark' ? 'light' : 'dark' });
  render();
  return { el: h('div', { class: 'appearance-controls' }, theme), destroy: onAppearance(render) };
}
