// Resume playing: the last few games, on the desktop of a space that asks for
// it. The library itself is its own app now; this is the short way back into
// whatever was being played, one click from the home screen.

import * as bridge from './bridge';
import { h, RESUME_EVENT } from './dom';
import { type Game, gameArt, gameCommand, type GameLibrary, launchGame, libraryPreferences, nativeGames, playedAt, runningWindow, saveLibraryPreferences, sourceLabel } from './games';
import { icon } from './icons';
import { store } from './state';

/** One scan shared by every card this page builds; a space switch re-renders
 *  the card, which should not start the scanner again. */
const cache: { games: Game[]; at: number; failed: boolean; pending?: Promise<void> } = { games: [], at: 0, failed: false };
/** The launchers' own "last played" only moves when a game ends, so a scan a
 *  minute old is as good as a new one. */
const FRESH_MS = 60_000;
const SHOWN = 4;

function scan(): Promise<void> {
  cache.pending ??= gameCommand<GameLibrary>('scan')
    .then((r) => { cache.games = r.games; cache.failed = false; })
    .catch(() => { cache.failed = true; })
    .finally(() => { cache.at = Date.now(); cache.pending = undefined; });
  return cache.pending;
}

function ago(secs: number): string {
  if (!secs) return 'Not played yet';
  const d = Date.now() / 1000 - secs;
  if (d < 90) return 'Played just now';
  if (d < 3600) return `Played ${Math.round(d / 60)} minutes ago`;
  if (d < 86400) return `Played ${Math.round(d / 3600)} hours ago`;
  if (d < 2 * 86400) return 'Played yesterday';
  if (d < 30 * 86400) return `Played ${Math.round(d / 86400)} days ago`;
  return `Played ${new Date(secs * 1000).toLocaleDateString(undefined, { month: 'short', day: 'numeric', year: 'numeric' })}`;
}

export function openLibrary(): Promise<unknown> {
  // One library: an open one (on any space) comes forward instead of a second.
  const open = store.state.allWindows.find((w) => w.app_id === 'mindos-library');
  return open ? bridge.call('windows.focus', { id: open.id }) : bridge.call('shell.openApp', { name: 'library' });
}

export function resumeCard(report: (text: string) => void): { el: HTMLElement; destroy: () => void } {
  let alive = true;
  let pending = '';
  const body = h('div', { class: 'resume-body' });
  const el = h('section', { class: 'resume-card', 'aria-label': 'Resume playing', hidden: true },
    h('header', { class: 'gaming-panel-title' }, h('h2', {}, 'Resume playing'),
      h('button', { class: 'gaming-text-action', onclick: () => void openLibrary().catch((e) => report(bridge.reason(e))) }, 'Open library', icon('arrow-right', 12))),
    body);

  const recent = () => {
    const prefs = libraryPreferences();
    return [...cache.games, ...nativeGames()]
      .filter((g) => playedAt(g, prefs) > 0 || runningWindow(g))
      .sort((a, b) => Number(!!runningWindow(b)) - Number(!!runningWindow(a)) || playedAt(b, prefs) - playedAt(a, prefs))
      .slice(0, SHOWN);
  };
  const play = async (game: Game) => {
    if (pending) return;
    pending = game.id; render();
    try {
      const result = await launchGame(game);
      const prefs = libraryPreferences();
      prefs.launched[game.id] = Math.floor(Date.now() / 1000);
      saveLibraryPreferences(prefs);
      if (result === 'launched') report(`Starting ${game.name}…`);
    } catch (e) { report(bridge.reason(e)); }
    finally { pending = ''; if (alive) render(); }
  };

  // Rebuilt only when what it shows changes: window events are frequent and
  // every rebuild decodes the artwork again.
  let key = '';
  const render = () => {
    const games = recent();
    const next = JSON.stringify([games.map((g) => [g.id, g.name, g.art, !!runningWindow(g)]), pending, cache.failed, cache.at > 0, libraryPreferences().launched]);
    if (next === key) return;
    key = next;
    // Nothing to resume, or no gaming tools to ask: the card is not worth its room.
    if (!games.length) {
      el.hidden = !(cache.at > 0 && !cache.failed);
      body.replaceChildren(h('p', { class: 'resume-empty' }, icon('gamepad', 18), 'Games you play show up here.'));
      return;
    }
    el.hidden = false;
    const [top, ...rest] = games;
    const running = runningWindow(top);
    const prefs = libraryPreferences();
    body.replaceChildren(
      h('div', { class: 'resume-hero' }, gameArt(top, 'resume-hero-art'),
        h('div', { class: 'resume-hero-copy' },
          h('span', { class: 'gaming-meta' }, running ? 'Running' : `${sourceLabel[top.source] || top.source} · ${ago(playedAt(top, prefs))}`),
          h('strong', {}, top.name),
          h('button', { class: 'btn primary resume-play', disabled: pending === top.id, onclick: () => void play(top) },
            icon(running ? 'refresh' : 'gamepad', 15), pending === top.id ? 'Opening…' : running ? 'Return to game' : 'Resume'))),
      ...(rest.length ? [h('div', { class: 'resume-more' }, ...rest.map((g) => {
        const on = runningWindow(g);
        return h('button', { class: 'resume-tile', title: `${on ? 'Return to' : 'Play'} ${g.name}`, disabled: pending === g.id, onclick: () => void play(g) },
          gameArt(g, 'resume-tile-art'),
          h('span', { class: 'resume-tile-copy' }, h('strong', {}, g.name), h('span', { class: 'gaming-meta' }, on ? 'Running' : ago(playedAt(g, prefs)))));
      }))] : []));
  };

  const refresh = () => {
    // A game has the machine: the list can wait until the desktop is back.
    if (store.state.game || Date.now() - cache.at < FRESH_MS) return render();
    void scan().then(() => { if (alive) render(); });
  };
  let running = store.state.allWindows.map((w) => w.app_id).join(' ');
  const windows = () => {
    const now = store.state.allWindows.map((w) => w.app_id).join(' ');
    if (now === running) return;
    running = now; render();
  };
  window.addEventListener(RESUME_EVENT, refresh);
  const offs = [store.on('windows', windows), store.on('apps', render), store.on('layout', render)];
  render();
  refresh();
  return { el, destroy: () => { alive = false; window.removeEventListener(RESUME_EVENT, refresh); offs.forEach((off) => off()); } };
}
