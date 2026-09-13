import * as bridge from './bridge';
import { h } from './dom';
import { store } from './state';
import type { RunResult, WindowInfo } from './types';

export interface Game {
  id: string;
  name: string;
  source: string;
  path?: string;
  art?: string;
  lastPlayed?: number;
  desktopId?: string;
}
export interface GameLibrary { games: Game[]; warnings: string[] }
export const sourceLabel: Record<string, string> = { steam: 'Steam', heroic: 'Epic · Heroic', gog: 'GOG · Heroic', lutris: 'Lutris', native: 'Desktop' };

/** Shown whenever the optional gaming tools are not on the system. */
const NO_TOOLS = 'Gaming tools are not installed yet. Settings \u203a Games installs them.';

export async function gameCommand<T>(...args: string[]): Promise<T> {
  let r: RunResult;
  try {
    r = await bridge.call<RunResult>('shell.run', { argv: ['mindos-games', ...args] });
  } catch (e) {
    // mindos-gaming is an optional package: say that, rather than repeating
    // the operating system's own "no such file" at the person using it.
    throw new Error(/no such file|not found/i.test(bridge.reason(e)) ? NO_TOOLS : bridge.reason(e));
  }
  if (!r.ok || r.json == null) throw new Error((r.json as { error?: string })?.error || r.stderr.trim() || NO_TOOLS);
  return r.json as T;
}

export function runningWindow(game: Game): WindowInfo | undefined {
  const app = game.desktopId ? store.state.apps.find((a) => a.id === game.desktopId) : undefined;
  const ids = game.id.startsWith('steam:') ? [`steam_app_${game.id.slice(6)}`]
    : app ? [app.id.replace(/\.desktop$/, ''), ...(app.wmClass ? [app.wmClass] : [])] : [];
  // Every space's windows: a game left running on another space is still
  // running, and focusing it takes the desktop there.
  return store.state.allWindows.find((w) => ids.some((id) => w.app_id.toLowerCase() === id.toLowerCase()));
}

export async function launchGame(game: Game): Promise<'focused' | 'launched'> {
  const running = runningWindow(game);
  if (running) {
    await bridge.call('windows.focus', { id: running.id });
    return 'focused';
  }
  if (game.desktopId) await bridge.call('apps.launch', { id: game.desktopId });
  else await gameCommand('launch', game.id);
  return 'launched';
}

export function nativeGames(): Game[] {
  return store.state.apps.filter((a) => a.categories.includes('Game')
    && !/steam|lutris|heroic|prism|win-open|protontricks|winetricks|mangohud|goverlay|gamemode|mindos-(library|gaming)/i.test(a.id)
    && !/steam:\/\/(run|rungameid)|heroic:\/\/|lutris:rungame/i.test(a.exec))
    .map((a) => ({ id: `desktop:${a.id}`, name: a.name, source: 'native', desktopId: a.id }));
}

const KEY = 'mindos.library.v1';
export type Preferences = { favorites: string[]; launched: Record<string, number> };
export function libraryPreferences(): Preferences {
  try {
    const p = location.protocol === 'mindos:' ? store.state.layout.desktop.library || {} : JSON.parse(localStorage.getItem(KEY) || '{}');
    return { favorites: Array.isArray(p.favorites) ? p.favorites.filter((id: unknown) => typeof id === 'string') : [],
      launched: p.launched && typeof p.launched === 'object' ? p.launched : {} };
  } catch { return { favorites: [], launched: {} }; }
}
export function saveLibraryPreferences(p: Preferences): void {
  if (location.protocol === 'mindos:') {
    void store.updateLayout((layout) => { layout.desktop.library = { favorites: [...p.favorites], launched: { ...p.launched } }; });
    return;
  }
  try { localStorage.setItem(KEY, JSON.stringify(p)); } catch { /* Storage may be disabled. */ }
}

/** When a game was last played: the launcher's own record, or when MindOS last started it. */
export function playedAt(game: Game, prefs: Preferences = libraryPreferences()): number {
  return Math.max(game.lastPlayed || 0, Number(prefs.launched[game.id]) || 0);
}

/** Box art for a game, with a drawn placeholder under it while (or if) it does not load. */
export function gameArt(game: Game, cls: string): HTMLElement {
  const box = h('div', { class: `game-art ${cls}`, 'aria-hidden': 'true' },
    h('span', { class: 'game-art-index' }, sourceLabel[game.source] || game.source),
    h('span', { class: 'game-art-letter' }, game.name.split(/\s+/).map((s) => s[0]).slice(0, 2).join('')),
    h('span', { class: 'game-art-cross' }, '+'));
  let hash = 0;
  for (const ch of game.id) hash = (hash * 31 + ch.charCodeAt(0)) | 0;
  box.style.setProperty('--art-hue', String(Math.abs(hash) % 360));
  const steamId = game.id.match(/^steam:(\d+)$/)?.[1];
  const fallback = steamId ? `https://cdn.cloudflare.steamstatic.com/steam/apps/${steamId}/header.jpg` : '';
  if (game.art || fallback) {
    const img = h('img', { src: game.art ? `mindos://shell/file/${encodeURIComponent(game.art)}` : fallback, alt: '', loading: 'lazy' });
    img.onerror = () => {
      if (fallback && img.src !== fallback) img.src = fallback;
      else img.remove();
    };
    box.append(img);
  }
  return box;
}
