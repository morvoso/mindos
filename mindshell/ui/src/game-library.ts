import * as bridge from './bridge';
import { h } from './dom';
import { Game, GameLibrary, gameCommand, launchGame, libraryPreferences, nativeGames, runningWindow, saveLibraryPreferences, sourceLabel } from './games';
import { icon } from './icons';
import { store } from './state';
import { openGaming, openCompanion, play as gamingRequest, type GameMeta, type Session } from './gaming';

export function renderGameLibrary(root: HTMLElement, close?: () => void): () => void {
  root.classList.add('game-library');
  let alive = true, loading = false, launchPending = false;
  let scanned: Game[] = [], selected = '', source = 'all', filter = 'all';
  let prefs = libraryPreferences();
  let loaded = false;
  let details: { metadata: Record<string, GameMeta>; storage: Record<string, unknown>; sessions: Session[] } = { metadata: {}, storage: {}, sessions: [] };
  const summary = h('span', { class: 'gaming-meta' }, 'Reading local libraries');
  const message = h('p', { class: 'gaming-message', role: 'status', 'aria-live': 'polite', hidden: true });
  const search = h('input', { class: 'input game-search', type: 'search', placeholder: 'Find a game…', 'aria-label': 'Search game library' });
  const sources = h('select', { class: 'select', 'aria-label': 'Filter by launcher' },
    ...[['all', 'All sources'], ...Object.entries(sourceLabel)].map(([value, label]) => h('option', { value }, label)));
  const sort = h('select', { class: 'select', 'aria-label': 'Sort games' }, h('option', { value: 'recent' }, 'Last played'), h('option', { value: 'name' }, 'Name A–Z'));
  const refresh = h('button', { class: 'btn', 'aria-label': 'Refresh game library', onclick: () => void load() }, icon('refresh', 14));
  const hero = h('div', { class: 'game-feature' });
  const grid = h('div', { class: 'game-grid' });
  const count = h('span', { class: 'gaming-meta', role: 'status' });
  const tabs = h('div', { class: 'game-tabs', 'aria-label': 'Library view' });
  const footer = h('footer', { class: 'gaming-panel-footer' });
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
    if (game.art) {
      // Only cached local artwork. The shell never downloads covers on login.
      const img = h('img', { src: `mindos://shell/file/${encodeURIComponent(game.art)}`, alt: '', loading: 'lazy' });
      img.onerror = () => img.remove();
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
      details.sessions.filter(s => s.game === game.id).forEach(s => { s.suspended = false; });
      prefs.launched[game.id] = Math.floor(Date.now() / 1000);
      saveLibraryPreferences(prefs);
      report(result === 'focused' ? `Returned to ${game.name}.` : `${game.name} requested in ${sourceLabel[game.source] || game.source}. The launcher handles startup.`);
    } catch (e) { report(String(e), true); }
    finally { launchPending = false; if (alive) render(); }
  };
  const renderHero = (game?: Game) => {
    if (!game) { hero.hidden = true; return; }
    hero.hidden = false;
    const running = runningWindow(game);
    const favorite = prefs.favorites.includes(game.id);
    const held = details.sessions.some(s => s.game === game.id && s.active && s.suspended);
    const meta = details.metadata[game.id];
    hero.replaceChildren(art(game, 'game-feature-art'), h('div', { class: 'game-feature-copy' },
      h('span', { class: 'game-feature-state gaming-meta' }, h('i'), held ? 'Held in memory · ready to resume' : running ? 'Running · return to your game' : details.storage[game.id] ? 'Cold storage · linked & playable' : 'Installed · ready to launch'),
      h('h1', {}, game.name),
      h('p', { class: 'gaming-meta' }, `${sourceLabel[game.source] || game.source} / ${meta?.completion != null ? `${meta.completion}% · ${meta.chapter || 'Progress tracked'}` : 'Local library'}`),
      h('div', { class: 'game-feature-actions' },
        h('button', { class: 'btn primary game-play', disabled: launchPending, onclick: () => void play(game) }, icon(running ? 'refresh' : 'gamepad', 16), launchPending ? 'Opening…' : held ? 'Resume' : running ? 'Return to game' : 'Play'),
        h('button', { class: 'btn', onclick: () => void command(() => settings()) }, icon('sliders', 14), 'Tuning'),
        h('button', { class: 'btn', onclick: () => void command(() => openGaming('sessions', game.id)) }, 'Session'),
        h('button', { class: 'btn', onclick: () => void command(() => openGaming('saves', game.id)) }, 'Saves'),
        h('button', { class: 'btn', onclick: () => void command(() => openCompanion(game.id)) }, 'Companion'),
        game.path ? h('button', { class: 'btn', title: 'Open game folder', 'aria-label': 'Open game folder', onclick: () => void command(() => bridge.call('fs.open', { path: game.path })) }, icon('folder', 14)) : null,
        h('button', { class: 'btn game-favorite', title: 'Favorite', 'aria-label': `Favorite ${game.name}`, 'aria-pressed': String(favorite), onclick: () => {
          prefs.favorites = favorite ? prefs.favorites.filter((id) => id !== game.id) : [...prefs.favorites, game.id];
          saveLibraryPreferences(prefs); render();
        } }, icon('star', 14)))));
  };
  const render = () => {
    const games = allGames();
    const needle = search.value.trim().toLocaleLowerCase();
    const visible = games.filter((g) => (source === 'all' || g.source === source)
      && (filter !== 'favorites' || prefs.favorites.includes(g.id))
      && `${g.name} ${sourceLabel[g.source] || g.source}`.toLocaleLowerCase().includes(needle));
    if (!games.some((g) => g.id === selected)) selected = games[0]?.id || '';
    summary.textContent = `${games.length} titles · ${new Set(games.map((g) => g.source)).size} sources`;
    count.textContent = `${visible.length} ${visible.length === 1 ? 'title' : 'titles'}`;
    renderHero(games.find((g) => g.id === selected));
    tabs.replaceChildren(...[['all', 'Installed'], ['favorites', 'Favorites']].map(([value, label]) =>
      h('button', { class: 'game-tab', 'aria-pressed': String(filter === value), onclick: () => { filter = value; render(); } }, label)));
    grid.replaceChildren(...visible.map((game, i) => h('button', {
      class: `game-tile${game.id === selected ? ' selected' : ''}`, 'aria-label': `Select ${game.name}`, 'aria-pressed': String(game.id === selected),
      dataset: { game: game.id }, onclick: () => { selected = game.id; render(); grid.querySelector<HTMLButtonElement>(`[data-index="${i}"]`)?.focus(); },
    }, art(game, 'game-tile-art'), h('strong', {}, game.name), h('span', { class: 'gaming-meta' }, runningWindow(game) ? '● Running' : sourceLabel[game.source] || game.source))));
    grid.querySelectorAll<HTMLElement>('.game-tile').forEach((el, i) => { el.dataset.index = String(i); });
    if (!visible.length) grid.append(h('div', { class: 'game-empty' }, icon('gamepad', 32),
      h('h3', {}, loading && !loaded ? 'Reading your libraries…' : games.length ? 'No matching games' : 'No installed games'),
      h('p', {}, games.length ? 'Try another search, source or view.' : 'Install a game in Steam, Heroic or Lutris, then refresh.'),
      !games.length ? h('button', { class: 'btn', onclick: () => void command(() => settings()) }, 'Set up gaming tools', icon('arrow-right', 14)) : null));
    footer.replaceChildren(
      h('button', { class: 'gaming-text-action', onclick: () => void command(() => bridge.call('mind.open', { text: 'Help me troubleshoot a game on Linux.' })) }, 'Ask Mind', icon('arrow-right', 12)));
  };
  const load = async () => {
    if (loading) return;
    loading = true; refresh.disabled = true; root.setAttribute('aria-busy', 'true'); render();
    try {
      const result = await gameCommand<GameLibrary>('scan');
      if (!alive) return;
      scanned = result.games;
      try { details = await gamingRequest<typeof details>('library.state', { games: nativeGames().map(g => g.id) }); } catch { /* Library works without the optional gaming service. */ }
      loaded = true;
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
  const storage = () => { prefs = libraryPreferences(); render(); };
  window.addEventListener('storage', storage);
  const offs = [store.on('apps', render), store.on('layout', storage), store.on('windows', () => renderHero(allGames().find((g) => g.id === selected)))];
  void load();
  return () => { alive = false; offs.forEach((off) => off()); window.removeEventListener('storage', storage); root.removeEventListener('keydown', keyboard); };
}
