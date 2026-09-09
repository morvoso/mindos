// The colour scheme. Seven colours per theme (dark and light) drive every
// token in app.css; presets are named starting points and a saved palette is
// one the user made. Native preferences live in layout.json so the host can
// share them between shell processes; browser previews use localStorage and
// the greeter's choice is temporary.
import { icon } from './icons';
import { h } from './dom';
import { store } from './state';
import { frostedWallpaper } from './static-wallpaper';
import type { AppearancePalette, SavedPalette } from './types';

export type Appearance = {
  theme: 'dark' | 'light';
  dark: AppearancePalette;
  light: AppearancePalette;
  /** Name of the preset the palettes came from, while they are unchanged. */
  preset?: string;
  /** Palettes the user saved, newest last. */
  saved?: SavedPalette[];
};
const KEY = 'mindos.appearance.v1';
const listeners = new Set<() => void>();
const native = () => location.protocol === 'mindos:' && document.documentElement.dataset.kind !== 'greeter';

/** The named starting points. The first one is what a fresh install looks like. */
export const PRESETS: { id: string; name: string; note: string; dark: AppearancePalette; light: AppearancePalette }[] = [
  {
    id: 'manjaro', name: 'Manjaro Green', note: 'Neutral graphite with the Manjaro green',
    dark: { accent: '#35bf5c', background: '#16181c', surface: '#1c1f24', surfaceStrong: '#2a2f36', border: '#333941', text: '#e9edf1', muted: '#9aa4b0' },
    light: { accent: '#249147', background: '#f4f5f7', surface: '#ffffff', surfaceStrong: '#e6e9ec', border: '#d2d7dd', text: '#1a1d21', muted: '#5c6570' },
  },
  {
    id: 'graphite', name: 'Graphite Cyan', note: 'The original MindOS cyan',
    dark: { accent: '#67dce5', background: '#0d1016', surface: '#141820', surfaceStrong: '#242c38', border: '#2a3442', text: '#edf2f8', muted: '#adb8c9' },
    light: { accent: '#0f7b86', background: '#f2f4f7', surface: '#ffffff', surfaceStrong: '#e3e8ee', border: '#cfd6de', text: '#171b21', muted: '#57616e' },
  },
  {
    id: 'ember', name: 'Ember', note: 'Warm grey with an amber highlight',
    dark: { accent: '#ff9d3d', background: '#171513', surface: '#201d1a', surfaceStrong: '#2e2a26', border: '#3b3630', text: '#f0ece7', muted: '#b0a79c' },
    light: { accent: '#b45f06', background: '#f6f4f1', surface: '#ffffff', surfaceStrong: '#eae5df', border: '#d8d2ca', text: '#1f1c19', muted: '#665f57' },
  },
  {
    id: 'violet', name: 'Nebula', note: 'Cool grey with a violet highlight',
    dark: { accent: '#a78bfa', background: '#131318', surface: '#1a1a22', surfaceStrong: '#272733', border: '#33333f', text: '#eceaf3', muted: '#a4a1b4' },
    light: { accent: '#6d43d6', background: '#f5f4f8', surface: '#ffffff', surfaceStrong: '#e8e6ef', border: '#d5d2df', text: '#1a1922', muted: '#5f5b6e' },
  },
  {
    id: 'ocean', name: 'Deep Ocean', note: 'Slate blue with a bright teal highlight',
    dark: { accent: '#2ec4c4', background: '#101519', surface: '#161d23', surfaceStrong: '#232d36', border: '#2c3843', text: '#e6eef3', muted: '#98a8b4' },
    light: { accent: '#0d7d7d', background: '#f1f5f7', surface: '#ffffff', surfaceStrong: '#e1e9ee', border: '#ccd8df', text: '#131b21', muted: '#54636e' },
  },
];
export const defaultPalettes: Record<'dark' | 'light', AppearancePalette> = { dark: { ...PRESETS[0].dark }, light: { ...PRESETS[0].light } };
export const appearance: Appearance = { theme: 'dark', dark: { ...defaultPalettes.dark }, light: { ...defaultPalettes.light }, preset: PRESETS[0].id, saved: [] };

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

function parse(value: string): [number, number, number] {
  const n = value.slice(1);
  return [0, 2, 4].map((i) => parseInt(n.slice(i, i + 2), 16)) as [number, number, number];
}

function rgb(value: string): string {
  return parse(value).join(' ');
}

function toHex(c: [number, number, number]): string {
  return '#' + c.map((v) => Math.max(0, Math.min(255, Math.round(v))).toString(16).padStart(2, '0')).join('');
}

/** Blend two colours; `t` is how much of `b` ends up in the result. */
function mix(a: string, b: string, t: number): string {
  const [x, y] = [parse(a), parse(b)];
  return toHex([0, 1, 2].map((i) => x[i] + (y[i] - x[i]) * t) as [number, number, number]);
}

function luminance(value: string): number {
  const linear = (x: number) => (x <= .03928 ? x / 12.92 : ((x + .055) / 1.055) ** 2.4);
  const [r, g, b] = parse(value).map((v) => linear(v / 255));
  return .2126 * r + .7152 * g + .0722 * b;
}

/** Black or white, whichever reads on this colour. */
function contrast(value: string): string {
  return luminance(value) > .46 ? '#111418' : '#ffffff';
}

function apply(): void {
  document.documentElement.dataset.theme = appearance.theme;
  const p = appearance[appearance.theme];
  const light = appearance.theme === 'light';
  const root = document.documentElement.style;
  const set = (name: string, value: string) => root.setProperty(name, value);
  // Surfaces: the background is the darkest (or lightest) ground, and every
  // raised surface is a step towards the text colour so a hand-picked pair
  // still has depth.
  set('--void', mix(p.background, light ? '#ffffff' : '#000000', .5));
  set('--bg-0', p.background);
  set('--bg-1', p.surface);
  set('--bg-2', p.surfaceStrong);
  set('--bg-3', mix(p.surfaceStrong, p.text, .1));
  set('--hairline', p.border);
  set('--line-strong', mix(p.border, p.text, .3));
  set('--fg', p.text);
  set('--fg-dim', p.muted);
  set('--fg-faint', mix(p.muted, p.background, .42));
  set('--accent', p.accent);
  set('--accent-dim', mix(p.accent, p.background, .35));
  set('--accent-a', rgb(p.accent));
  set('--accent-fg', contrast(p.accent));
  set('--mind', p.accent);
  set('--mind-a', rgb(p.accent));
  set('--action', p.accent);
  set('--action-fg', contrast(p.accent));
  set('--text', p.text);
  set('--muted', p.muted);
  set('--border', p.border);
  // Glass: a tint of the surface over the blurred wallpaper, with highlights
  // and hairlines made of light (dark theme) or shade (light theme) so they
  // stay subtle whatever the user picked.
  const edge = light ? '0 0 0' : '255 255 255';
  set('--glass', rgb(p.surface));
  set('--glass-border', `rgb(${edge} / ${light ? .1 : .09})`);
  set('--glass-border-strong', `rgb(${edge} / ${light ? .17 : .16})`);
  set('--glass-hi', `rgb(255 255 255 / ${light ? .5 : .07})`);
  set('--surface', `rgb(${edge} / ${light ? .04 : .05})`);
  set('--surface-hover', `rgb(${edge} / ${light ? .075 : .09})`);
  set('--surface-strong', `rgb(${edge} / ${light ? .12 : .14})`);
  set('--glow-accent', `0 4px 18px rgb(${rgb(p.accent)} / ${light ? .18 : .22})`);
  set('--aurora', aurora(p, light));
  set('--frosted-wallpaper', frostedWallpaper(light, p.background, p.accent));
  document.documentElement.classList.toggle('software-rendered', store.state.outputs.some(o => o.software_rendering === true)
    || (location.protocol === 'mindos:' && !store.state.outputs.length));
  listeners.forEach((fn) => fn());
}

/** The built-in wallpaper: the palette's own colours, no animation. */
function aurora(p: AppearancePalette, light: boolean): string {
  const deep = mix(p.background, light ? '#ffffff' : '#000000', light ? .35 : .45);
  const glow = rgb(p.accent);
  return [
    `radial-gradient(ellipse 70% 60% at 78% 82%, rgb(${glow} / ${light ? .1 : .13}), transparent 62%)`,
    `radial-gradient(ellipse 60% 55% at 10% 4%, rgb(${rgb(mix(p.surfaceStrong, p.accent, .25))} / ${light ? .5 : .55}), transparent 70%)`,
    `linear-gradient(150deg, ${mix(p.surface, p.background, .3)}, ${p.background} 52%, ${deep})`,
  ].join(',');
}

function read(): void {
  try {
    const value = JSON.parse(localStorage.getItem(KEY) || '{}');
    fromValue(value);
  } catch { /* Restricted or damaged storage uses the defaults. */ }
  apply();
}

function fromValue(value: Partial<Appearance> | null | undefined): void {
  const v = value ?? {};
  const preset = PRESETS.find((x) => x.id === v.preset) ?? PRESETS[0];
  appearance.theme = v.theme === 'light' ? 'light' : 'dark';
  appearance.dark = palette(v.dark, preset.dark);
  appearance.light = palette(v.light, preset.light);
  appearance.preset = typeof v.preset === 'string' ? v.preset : undefined;
  appearance.saved = Array.isArray(v.saved)
    ? v.saved.filter((s) => s && typeof s.name === 'string').slice(0, 24).map((s) => ({
        name: String(s.name).slice(0, 40), dark: palette(s.dark, preset.dark), light: palette(s.light, preset.light),
      }))
    : [];
}

export function initAppearance(): void {
  read();
  const fromLayout = () => {
    if (!native()) return;
    fromValue(store.state.layout.desktop.appearance as Partial<Appearance>);
    apply();
  };
  fromLayout();
  store.on('layout', fromLayout);
  store.on('outputs', apply);
  window.addEventListener('storage', (e) => { if (!native() && (e.key === KEY || e.key === null)) read(); });
}

function persist(): void {
  try { localStorage.setItem(KEY, JSON.stringify(appearance)); } catch { /* Still works for this session. */ }
  apply();
  if (native()) void store.updateLayout((layout) => { layout.desktop.appearance = JSON.parse(JSON.stringify(appearance)); });
}

export function setAppearance(patch: Partial<Appearance>): void {
  Object.assign(appearance, patch);
  persist();
}

export function paletteFor(theme: 'dark' | 'light' = appearance.theme): AppearancePalette {
  return appearance[theme];
}

export function setPaletteColor(theme: 'dark' | 'light', key: keyof AppearancePalette, value: string): void {
  if (!/^#[0-9a-f]{6}$/i.test(value)) return;
  appearance[theme] = { ...appearance[theme], [key]: value };
  appearance.preset = undefined;
  persist();
}

/** Load a named starting point into both themes. */
export function usePreset(id: string): void {
  const preset = PRESETS.find((p) => p.id === id);
  if (!preset) return;
  appearance.dark = { ...preset.dark };
  appearance.light = { ...preset.light };
  appearance.preset = preset.id;
  persist();
}

export function savedPalettes(): SavedPalette[] {
  return appearance.saved ?? [];
}

/** Keep the current colours under a name (an existing name is replaced). */
export function savePalette(name: string): void {
  const clean = name.trim().slice(0, 40);
  if (!clean) return;
  const rest = savedPalettes().filter((s) => s.name.toLowerCase() !== clean.toLowerCase());
  appearance.saved = [...rest, { name: clean, dark: { ...appearance.dark }, light: { ...appearance.light } }].slice(-24);
  persist();
}

export function useSavedPalette(name: string): void {
  const found = savedPalettes().find((s) => s.name === name);
  if (!found) return;
  appearance.dark = { ...found.dark };
  appearance.light = { ...found.light };
  appearance.preset = undefined;
  persist();
}

export function deleteSavedPalette(name: string): void {
  appearance.saved = savedPalettes().filter((s) => s.name !== name);
  persist();
}

export function onAppearance(fn: () => void): () => void {
  listeners.add(fn);
  return () => { listeners.delete(fn); };
}

export function appearanceControls(): { el: HTMLElement; destroy: () => void } {
  const theme = h('button', { class: 'tool', 'aria-label': 'Use light theme' });
  const render = () => {
    theme.replaceChildren(icon(appearance.theme === 'dark' ? 'sun' : 'moon', 17));
    theme.title = `Use ${appearance.theme === 'dark' ? 'light' : 'dark'} theme`;
    theme.setAttribute('aria-label', `Use ${appearance.theme === 'dark' ? 'light' : 'dark'} theme`);
  };
  theme.onclick = () => setAppearance({ theme: appearance.theme === 'dark' ? 'light' : 'dark' });
  render();
  return { el: h('div', { class: 'appearance-controls' }, theme), destroy: onAppearance(render) };
}
