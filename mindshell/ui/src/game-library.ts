import * as bridge from './bridge';
import { h, reconcile } from './dom';
import { Game, GameLibrary, gameCommand, launchGame, libraryPreferences, nativeGames, runningWindow, saveLibraryPreferences, sourceLabel } from './games';
import { icon } from './icons';
import { store } from './state';
import { openGaming, play as gamingRequest, type GameMeta } from './gaming';

type Details = { metadata: Record<string, GameMeta>; storage: Record<string, unknown> };

/** The last scan, kept for the life of the page. Switching between the gaming
 *  and productivity desktops remounts the library, which then paints from here
 *  at once instead of spawning the scanner again. A scan older than FRESH_MS,
 *  or one taken before the desktop entries changed, is repeated quietly behind
 *  the cached grid; the Refresh button always scans again. */
const cache: { scanned: Game[]; warnings: string[]; details: Details; native: string; selected: string; at: number } =
  { scanned: [], warnings: [], details: { metadata: {}, storage: {} }, native: '', selected: '', at: 0 };
const FRESH_MS = 5 * 60_000;
const nativeKey = () => nativeGames().map((g) => g.id).join(' ');

export function renderGameLibrary(root: HTMLElement, close?: () => void): () => void {
  root.classList.add('game-library');
  let alive = true, loading = false, launchPending = false;
  let scanned = cache.scanned, selected = cache.selected, source = 'all', filter = 'all';
  let prefs = libraryPreferences();
  let loaded = cache.at > 0;
  let details = cache.details;
  const summary = h('span', { class: 'gaming-meta' }, 'Reading local libraries');
  const message = h('p', { class: 'gaming-message', role: 'status', 'aria-live': 'polite', hidden: true });
  const search = h('input', { class: 'input game-search', type: 'search', placeholder: 'Find a game…', 'aria-label': 'Search game library' });
  const sources = h('select', { class: 'select', 'aria-label': 'Filter by launcher' },
    ...[['all', 'All sources'], ...Object.entries(sourceLabel)].map(([value, label]) => h('option', { value }, label)));
  const sort = h('select', { class: 'select', 'aria-label': 'Sort games' }, h('option', { value: 'recent' }, 'Last played'), h('option', { value: 'name' }, 'Name A–Z'));
  const refresh = h('button', { class: 'btn', 'aria-label': 'Refresh game library', onclick: () => void load() }, icon('refresh', 14));
  const hero = h('div', { class: 'game-feature' });
  const grid = h('div', { class: 'game-grid' });
  const empty = h('div', { class: 'game-empty' });
  const count = h('span', { class: 'gaming-meta', role: 'status' });
  const tabs = h('div', { class: 'game-tabs', 'aria-label': 'Library view' });
  const footer = h('footer', { class: 'gaming-panel-footer' },
    h('button', { class: 'gaming-text-action', onclick: () => void command(() => bridge.call('mind.open', { text: 'Help me troubleshoot a game on Linux.' })) }, 'Ask Mind', icon('arrow-right', 12)));
  const heading = h('header', { class: 'gaming-panel-title' }, h('h2', {}, 'Library'), summary,
    close ? h('button', { class: 'gaming-close', title: 'Show desktop', 'aria-label': 'Close library', onclick: close }, icon('x', 14)) : null);
  root.append(heading, h('div', { class: 'game-library-body' },
    h('div', { class: 'game-toolbar' }, search, sources, sort, refresh), message, hero,
    h('div', { class: 'game-section-title' }, tabs, count), grid), footer);

  const settings = (page = 'games') => bridge.call('shell.openApp', { name: 'settings', page });
  const report = (text: string, error = false) => {
    if (!alive) return;
    message.textContent = text;
    message.hidden = !text;
    message.dataset.error = String(error);
  };
  const command = async (fn: () => Promise<unknown>) => {
    try { await fn(); } catch (e) { report(String(e), true); }
  };
  const allGames = () => {
    const games = [...scanned, ...nativeGames()];
    return games.sort((a, b) => sort.value === 'name' ? a.name.localeCompare(b.name)
      : Math.max(b.lastPlayed || 0, Number(prefs.launched[b.id]) || 0) - Math.max(a.lastPlayed || 0, Number(prefs.launched[a.id]) || 0) || a.name.localeCompare(b.name));
  };
  const art = (game: Game, cls: string) => {
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
  };
  const play = async (game: Game) => {
    if (launchPending) return;
    launchPending = true;
    renderHero(game);
    try {
      const result = await launchGame(game);
      prefs.launched[game.id] = Math.floor(Date.now() / 1000);
      saveLibraryPreferences(prefs);
      report(result === 'focused' ? `Returned to ${game.name}.` : `${game.name} requested in ${sourceLabel[game.source] || game.source}. The launcher handles startup.`);
    } catch (e) { report(String(e), true); }
    finally { launchPending = false; if (alive) render(); }
  };
  // The hero is rebuilt only when what it shows changes; window and layout
  // events are frequent, and each rebuild decodes the artwork again.
  let heroKey = '';
  const renderHero = (game?: Game) => {
    if (!game) { hero.hidden = true; heroKey = ''; return; }
    const running = runningWindow(game);
    const favorite = prefs.favorites.includes(game.id);
    const meta = details.metadata[game.id];
    const key = [game.id, game.name, game.source, game.path || '', game.art || '', !!running, favorite, launchPending, JSON.stringify(meta ?? null), !!details.storage[game.id]].join(' ');
    if (key === heroKey) return;
    heroKey = key;
    hero.hidden = false;
    hero.replaceChildren(art(game, 'game-feature-art'), h('div', { class: 'game-feature-copy' },
      h('span', { class: 'game-feature-state gaming-meta' }, h('i'), running ? 'Running · return to your game' : details.storage[game.id] ? 'Cold storage · linked & playable' : 'Installed · ready to launch'),
      h('h1', {}, game.name),
      h('p', { class: 'gaming-meta' }, `${sourceLabel[game.source] || game.source} / ${meta?.completion != null ? `${meta.completion}% · ${meta.chapter || 'Progress tracked'}` : 'Local library'}`),
      h('div', { class: 'game-feature-actions' },
        h('button', { class: 'btn primary game-play', disabled: launchPending, onclick: () => void play(game) }, icon(running ? 'refresh' : 'gamepad', 16), launchPending ? 'Opening…' : running ? 'Return to game' : 'Play'),
        h('button', { class: 'btn', onclick: () => void command(() => settings()) }, icon('sliders', 14), 'Tuning'),
        h('button', { class: 'btn', onclick: () => void command(() => openGaming('saves', game.id)) }, 'Saves'),
        game.path ? h('button', { class: 'btn', title: 'Open game folder', 'aria-label': 'Open game folder', onclick: () => void command(() => bridge.call('fs.open', { path: game.path })) }, icon('folder', 14)) : null,
        h('button', { class: 'btn game-favorite', title: 'Favorite', 'aria-label': `Favorite ${game.name}`, 'aria-pressed': String(favorite), onclick: () => {
          prefs.favorites = favorite ? prefs.favorites.filter((id) => id !== game.id) : [...prefs.favorites, game.id];
          saveLibraryPreferences(prefs); render();
        } }, icon('star', 14)))));
  };
  // Tiles are keyed by game and kept across renders: a search keystroke or a
  // selection moves and relabels them instead of rebuilding the grid.
  const tile = (game: Game) => {
    const el = h('button', { class: 'game-tile', dataset: { game: game.id }, onclick: () => { selected = el.dataset.game || game.id; render(); el.focus(); } },
      art(game, 'game-tile-art'), h('strong', {}, game.name), h('span', { class: 'gaming-meta' }));
    return el;
  };
  const updateTile = (el: HTMLElement, game: Game) => {
    const on = game.id === selected;
    if (el.classList.contains('selected') !== on) el.classList.toggle('selected', on);
    if (el.getAttribute('aria-pressed') !== String(on)) el.setAttribute('aria-pressed', String(on));
    const label = `Select ${game.name}`;
    if (el.getAttribute('aria-label') !== label) el.setAttribute('aria-label', label);
    const name = el.children[1], state = el.lastElementChild;
    if (name && name.textContent !== game.name) name.textContent = game.name;
    const text = runningWindow(game) ? '● Running' : sourceLabel[game.source] || game.source;
    if (state && state.textContent !== text) state.textContent = text;
  };
  let tabsFilter = '';
  const render = () => {
    const games = allGames();
    const needle = search.value.trim().toLocaleLowerCase();
    const visible = games.filter((g) => (source === 'all' || g.source === source)
      && (filter !== 'favorites' || prefs.favorites.includes(g.id))
      && `${g.name} ${sourceLabel[g.source] || g.source}`.toLocaleLowerCase().includes(needle));
    if (!games.some((g) => g.id === selected)) selected = games[0]?.id || '';
    cache.selected = selected;
    const text = `${games.length} titles · ${new Set(games.map((g) => g.source)).size} sources`;
    if (summary.textContent !== text) summary.textContent = text;
    const total = `${visible.length} ${visible.length === 1 ? 'title' : 'titles'}`;
    if (count.textContent !== total) count.textContent = total;
    renderHero(games.find((g) => g.id === selected));
    if (tabsFilter !== filter) {
      tabsFilter = filter;
      tabs.replaceChildren(...[['all', 'Installed'], ['favorites', 'Favorites']].map(([value, label]) =>
        h('button', { class: 'game-tab', 'aria-pressed': String(filter === value), onclick: () => { filter = value; render(); } }, label)));
    }
    empty.remove();
    reconcile(grid, visible, (g) => g.id, tile, updateTile);
    grid.querySelectorAll<HTMLElement>('.game-tile').forEach((el, i) => { if (el.dataset.index !== String(i)) el.dataset.index = String(i); });
    if (!visible.length) {
      empty.replaceChildren(icon('gamepad', 32),
        h('h3', {}, loading && !loaded ? 'Reading your libraries…' : games.length ? 'No matching games' : 'No installed games'),
        h('p', {}, games.length ? 'Try another search, source or view.' : 'Install a game in Steam, Heroic or Lutris, then refresh.'));
      if (!games.length) empty.append(h('button', { class: 'btn', onclick: () => void command(() => settings()) }, 'Set up gaming tools', icon('arrow-right', 14)));
      grid.append(empty);
    }
  };
  /** Scan the launchers. `quiet` keeps the cached grid on screen meanwhile. */
  const load = async (quiet = false) => {
    if (loading) return;
    loading = true; refresh.disabled = true; root.setAttribute('aria-busy', 'true');
    if (!quiet) render();
    try {
      const result = await gameCommand<GameLibrary>('scan');
      // The scan is worth keeping even after this view has gone.
      cache.scanned = result.games; cache.warnings = result.warnings; cache.at = Date.now(); cache.native = nativeKey();
      try { cache.details = await gamingRequest<Details>('library.state', { games: nativeGames().map((g) => g.id) }); } catch { /* Library works without the optional gaming service. */ }
      if (!alive) return;
      scanned = cache.scanned; details = cache.details; loaded = true;
      report(result.warnings.join(' '), result.warnings.length > 0);
    } catch (e) { report(String(e), true); }
    finally { loading = false; if (alive) { refresh.disabled = false; root.setAttribute('aria-busy', 'false'); render(); } }
  };
  search.oninput = render;
  sources.onchange = () => { source = sources.value; render(); };
  sort.onchange = render;
  const keyboard = (e: KeyboardEvent) => {
    if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 'k') { e.preventDefault(); search.focus(); }
    if (e.key === 'Escape' && search.value) { search.value = ''; render(); }
    const tile = (e.target as HTMLElement).closest<HTMLElement>('.game-tile');
    if (!tile || !['ArrowLeft', 'ArrowRight', 'ArrowUp', 'ArrowDown'].includes(e.key)) return;
    const tiles = [...grid.querySelectorAll<HTMLElement>('.game-tile')];
    const columns = Math.max(1, Math.round(grid.clientWidth / Math.max(tile.offsetWidth, 1)));
    const step = e.key === 'ArrowLeft' ? -1 : e.key === 'ArrowRight' ? 1 : e.key === 'ArrowUp' ? -columns : columns;
    const next = tiles[Math.max(0, Math.min(tiles.length - 1, tiles.indexOf(tile) + step))];
    if (next) { e.preventDefault(); next.focus(); }
  };
  root.addEventListener('keydown', keyboard);
  const storage = () => {
    const next = libraryPreferences();
    if (JSON.stringify(next) === JSON.stringify(prefs)) return;
    prefs = next; render();
  };
  window.addEventListener('storage', storage);
  // Window events arrive on every focus change; only a change in what runs matters here.
  let running = store.state.windows.map((w) => w.app_id).join(' ');
  const windows = () => {
    const now = store.state.windows.map((w) => w.app_id).join(' ');
    if (now === running) return;
    running = now; render();
  };
  const apps = () => {
    render();
    if (loaded && cache.native !== nativeKey()) void load(true);
  };
  const offs = [store.on('apps', apps), store.on('layout', storage), store.on('windows', windows)];
  if (loaded) report(cache.warnings.join(' '), cache.warnings.length > 0);
  render();
  if (!loaded || Date.now() - cache.at > FRESH_MS || cache.native !== nativeKey()) void load(loaded);
  return () => { alive = false; offs.forEach((off) => off()); window.removeEventListener('storage', storage); root.removeEventListener('keydown', keyboard); };
}
