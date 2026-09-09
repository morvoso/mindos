import * as bridge from './bridge';
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
  return store.state.windows.find((w) => ids.some((id) => w.app_id.toLowerCase() === id.toLowerCase()));
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
type Preferences = { favorites: string[]; launched: Record<string, number> };
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
