// Native preferences live in layout.json: the host watches it across shell
// processes. Browser previews use localStorage; greeter choices are temporary.
import { icon } from './icons';
import { h } from './dom';
import { store } from './state';
import { frostedWallpaper } from './static-wallpaper';
import type { AppearancePalette } from './types';

export type Appearance = { theme: 'dark' | 'light'; dark: AppearancePalette; light: AppearancePalette };
const KEY = 'mindos.appearance.v1';
const listeners = new Set<() => void>();
const native = () => location.protocol === 'mindos:' && document.documentElement.dataset.kind !== 'greeter';
export const defaultPalettes: Record<'dark' | 'light', AppearancePalette> = {
  dark: { accent: '#67dce5', background: '#0d1016', surface: '#141820', surfaceStrong: '#242c38', border: '#2a3442', text: '#edf2f8', muted: '#adb8c9' },
  light: { accent: '#b62e13', background: '#f3f2ef', surface: '#eeece7', surfaceStrong: '#d4cec6', border: '#c4bdb4', text: '#22201e', muted: '#544e48' },
};
export const appearance: Appearance = { theme: 'dark', dark: { ...defaultPalettes.dark }, light: { ...defaultPalettes.light } };

function hex(value: unknown, fallback: string): string {
  return typeof value === 'string' && /^#[0-9a-f]{6}$/i.test(value) ? value : fallback;
}

function palette(value: unknown, fallback: AppearancePalette): AppearancePalette {
  const input = value && typeof value === 'object' ? value as Partial<AppearancePalette> : {};
  return {
    accent: hex(input.accent, fallback.accent), background: hex(input.background, fallback.background),
    surface: hex(input.surface, fallback.surface), surfaceStrong: hex(input.surfaceStrong, fallback.surfaceStrong),
    border: hex(input.border, fallback.border), text: hex(input.text, fallback.text), muted: hex(input.muted, fallback.muted),
  };
}

function rgb(value: string): string {
  const n = value.slice(1);
  return `${parseInt(n.slice(0, 2), 16)} ${parseInt(n.slice(2, 4), 16)} ${parseInt(n.slice(4, 6), 16)}`;
}

function contrast(value: string): string {
  const n = value.slice(1);
  const [r, g, b] = [0, 2, 4].map((i) => parseInt(n.slice(i, i + 2), 16) / 255);
  const linear = (x: number) => x <= .03928 ? x / 12.92 : ((x + .055) / 1.055) ** 2.4;
  return .2126 * linear(r) + .7152 * linear(g) + .0722 * linear(b) > .46 ? '#111418' : '#ffffff';
}

function apply(): void {
  document.documentElement.dataset.theme = appearance.theme;
  const p = appearance[appearance.theme];
  const root = document.documentElement.style;
  root.setProperty('--void', p.background);
  root.setProperty('--bg-0', p.background);
  root.setProperty('--bg-1', p.surface);
  root.setProperty('--bg-2', p.surfaceStrong);
  root.setProperty('--bg-3', p.surfaceStrong);
  root.setProperty('--hairline', p.border);
  root.setProperty('--line-strong', p.border);
  root.setProperty('--fg', p.text);
  root.setProperty('--fg-dim', p.muted);
  root.setProperty('--fg-faint', p.muted);
  root.setProperty('--accent', p.accent);
  root.setProperty('--accent-dim', p.accent);
  root.setProperty('--accent-a', rgb(p.accent));
  root.setProperty('--mind', p.accent);
  root.setProperty('--mind-a', rgb(p.accent));
  root.setProperty('--action', p.accent);
  root.setProperty('--action-fg', contrast(p.accent));
  root.setProperty('--glass', rgb(p.surface));
  root.setProperty('--glass-border', `rgb(${rgb(p.border)} / .7)`);
  root.setProperty('--glass-border-strong', `rgb(${rgb(p.border)} / .95)`);
  root.setProperty('--surface', `rgb(${rgb(p.surface)} / .65)`);
  root.setProperty('--surface-hover', `rgb(${rgb(p.accent)} / .14)`);
  root.setProperty('--surface-strong', `rgb(${rgb(p.surfaceStrong)} / .9)`);
  document.documentElement.style.setProperty('--frosted-wallpaper', frostedWallpaper(appearance.theme === 'light'));
  document.documentElement.classList.toggle('software-rendered', store.state.outputs.some(o => o.software_rendering === true)
    || (location.protocol === 'mindos:' && !store.state.outputs.length));
  listeners.forEach((fn) => fn());
}

function read(): void {
  try {
    const value = JSON.parse(localStorage.getItem(KEY) || '{}');
    appearance.theme = value.theme === 'light' ? 'light' : 'dark';
    appearance.dark = palette(value.dark, defaultPalettes.dark);
    appearance.light = palette(value.light, defaultPalettes.light);
  } catch { /* Restricted or damaged storage uses the defaults. */ }
  apply();
}

export function initAppearance(): void {
  read();
  const fromLayout = () => {
    if (!native()) return;
    const saved = store.state.layout.desktop.appearance;
    appearance.theme = saved?.theme === 'light' ? 'light' : 'dark';
    appearance.dark = palette(saved?.dark, defaultPalettes.dark);
    appearance.light = palette(saved?.light, defaultPalettes.light);
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

export function paletteFor(theme: 'dark' | 'light' = appearance.theme): AppearancePalette {
  return appearance[theme];
}

export function setPaletteColor(theme: 'dark' | 'light', key: keyof AppearancePalette, value: string): void {
  if (!/^#[0-9a-f]{6}$/i.test(value)) return;
  appearance[theme] = { ...appearance[theme], [key]: value };
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
