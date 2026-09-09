import { h, gib } from '../dom';
import * as bridge from '../bridge';
import { gameCommand, nativeGames, sourceLabel, type Game, type GameLibrary } from '../games';
import { play, type GameMeta } from '../gaming';
import { store } from '../state';

const tabs = ['saves', 'storage', 'downloads', 'audio', 'history', 'connections', 'activity'];
const stamp = (s: number) => new Date(s * 1000).toLocaleString();
const text = (s: string) => h('p', { class: 'play-note' }, s);
const title = (s: string) => h('h2', {}, s);
const row = (...children: (Node | string)[]) => h('div', { class: 'play-row' }, ...children);
const panel = (...children: (Node | string)[]) => h('section', { class: 'play-card' }, ...children);
interface RunRecord { id: string; game: string; started: number; stats?: { avg_fps: number; p99_ms: number; stutters: number; points: number[] } }

export function renderGaming(root: HTMLElement, initial = 'downloads', gameId = ''): () => void {
  let page = tabs.includes(initial) ? initial : 'downloads';
  let games: Game[] = [], scanned: Game[] = [], chosen = gameId, generation = 0, busy = false;
  const select = h('select', { 'aria-label': 'Game', onchange: () => { chosen = select.value; void render(); } });
  const nav = h('nav', { class: 'play-tabs', 'aria-label': 'Gaming tools' });
  const body = h('div', { class: 'play-content' });
  const status = h('p', { class: 'play-status', role: 'status', 'aria-live': 'polite' });
  root.append(h('div', { class: 'gaming-app' }, h('header', { class: 'play-header' }, h('div', {}, h('span', { class: 'game-eyebrow' }, 'MINDOS / PLAY'), h('h1', {}, 'Gaming center')), select), nav, status, body));
  const button = (label: string, fn: () => Promise<unknown>, accent = false) => h('button', { class: `btn${accent ? ' primary' : ''}`, onclick: () => void act(fn) }, label);
  async function act(fn: () => Promise<unknown>): Promise<void> {
    if (busy) return; busy = true; status.textContent = 'Working…'; root.setAttribute('aria-busy', 'true');
    try { await fn(); status.textContent = 'Done.'; } catch (e) { status.textContent = bridge.reason(e); }
    finally { busy = false; root.removeAttribute('aria-busy'); }
  }
  function field(label: string, value = '', type = 'text'): [HTMLElement, HTMLInputElement] {
    const input = h('input', { class: 'input', type, value, autocomplete: type === 'password' ? 'new-password' : 'off' });
    return [h('label', { class: 'play-field' }, h('span', {}, label), input), input];
  }
  function requireGame(): Game { const game = games.find(g => g.id === chosen); if (!game) throw new Error('Install a game and refresh the library first.'); return game; }
  const rpc = <T>(action: string, data: Record<string, unknown> = {}) => play<T>(action, { game: chosen, ...data });
  async function render(): Promise<void> {
    const token = ++generation;
    nav.replaceChildren(...tabs.map(t => h('button', { class: t === page ? 'active' : '', 'aria-current': t === page ? 'page' : undefined, onclick: () => { page = t; void render(); } }, t)));
    body.replaceChildren(text('Loading…'));
    const content = h('div', { class: 'play-sections' });
    try {
      if (page === 'connections') {
        const cfg = await play<Record<string, string>>('config.get');
        const [cloudEl, cloud] = field('Synced save folder · existing absolute path', String(cfg.cloud_folder || ''));
        const [coldEl, cold] = field('Cold storage · existing absolute path on another drive', String(cfg.cold_folder || ''));
        content.append(panel(title('Storage locations'), text('Configure folders used for synced saves and cold game storage.'), cloudEl, coldEl,
          button('Save locations', async () => { await play('config.set', { settings: { cloud_folder: cloud.value.trim(), cold_folder: cold.value.trim() } }); await render(); }, true)));
      } else if (page === 'saves') {
        const game = requireGame(); const meta = await rpc<GameMeta>('metadata.get');
        const [fpsEl, fps] = field('Per-game FPS limit · 0 is unlimited', String(meta.fps_limit || 0), 'number'); fps.min = '0'; fps.max = '1000';
        const [progressEl, progress] = field('Completion %', String(meta.completion || 0), 'number'); progress.min = '0'; progress.max = '100';
        const [chapterEl, chapter] = field('Chapter / checkpoint', meta.chapter || '');
        content.append(panel(title(`${game.name} profile`), fpsEl, progressEl, chapterEl, button('Save game profile', async () => { await rpc('metadata.set', { settings: { fps_limit: Number(fps.value), completion: Number(progress.value), chapter: chapter.value } }); }, true)));
        const [pathEl, path] = field('Save folder · existing absolute path', meta.save_path || '');
        content.append(panel(title('Versioned saves'), text('Close the game before backup or restore. Each backup is verified. Restoring creates a backup of the current saves first.'), pathEl,
          row(button('Set save folder', async () => { await rpc('metadata.set', { settings: { save_path: path.value } }); await render(); }), button('Back up locally', async () => { await rpc('saves.backup'); await render(); }, true), button('Back up to synced folder', async () => { await rpc('saves.backup', { cloud: true }); await render(); }))));
        const saves = await rpc<{ id: string; time: number; cloud: boolean; files: number }[]>('saves.list');
        for (const save of saves) content.append(panel(h('strong', {}, stamp(save.time)), text(`${save.cloud ? 'Synced folder' : 'Local'} · ${save.files} files`), button('Review restore', async () => { content.append(panel(title('Restore this version?'), text(`Restore the backup from ${stamp(save.time)}. Current saves are preserved in a new backup.`), button('Restore saved version', async () => { await rpc('saves.restore', { revision: save.id }); await render(); }, true))); })));
        if (!saves.length) content.append(text('No backups for this game yet.'));
      } else if (page === 'storage') {
        requireGame(); const archive = await play<Record<string, { cold: string }>>('storage.list'); const restoring = !!archive[chosen];
        content.append(panel(title('Warm / cold storage'), text('Move a closed game to your configured cold drive while keeping its library path connected. Restore copies it back. Every regular file is checked with SHA-256; saves are managed separately.'),
          button(restoring ? 'Plan restore to warm drive' : 'Plan move to cold drive', async () => {
            const plan = await rpc<{ source: string; destination: string; bytes: number; free: number }>('storage.plan', { restore: restoring });
            content.append(panel(title('Review storage move'), text(`${plan.source} → ${plan.destination}`), text(`${gib(plan.bytes)} GiB to copy · ${gib(plan.free)} GiB free at destination`), button(restoring ? 'Restore game' : 'Move game', async () => { status.textContent = 'Copying and verifying files. Keep this window open; large games can take several minutes.'; await rpc('storage.move', { restore: restoring }); await render(); }, true)));
          }, true)));
        for (const [id, entry] of Object.entries(archive)) content.append(panel(h('strong', {}, games.find(g => g.id === id)?.name || id), text(`COLD · ${entry.cold}`)));
      } else if (page === 'downloads') {
        const queue = await play<{ items: { id: string; name: string; downloaded: number; total: number; percent: number }[]; note: string }>('downloads');
        content.append(panel(title('Downloads'), text(queue.note), button('Refresh progress', render)));
        for (const d of queue.items) content.append(panel(h('strong', {}, d.name), h('progress', { max: d.total, value: d.downloaded, 'aria-label': d.name }), text(`${d.percent}% · ${gib(d.downloaded)} / ${gib(d.total)} GiB`), button('Manage in Steam', () => bridge.call('shell.open', { uri: 'steam://nav/downloads' }))));
        if (!queue.items.length) content.append(text('No pending Steam downloads reported.'));
        content.append(panel(title('Other launchers'), ...store.state.apps.filter(a => /heroic|lutris/i.test(a.id)).map(a => button(`Open ${a.name}`, () => bridge.call('apps.launch', { id: a.id })))));
      } else if (page === 'audio') {
        content.append(panel(title('Per-app audio'), button('Refresh streams', render), button('Restore ducked audio', async () => { await play('audio.restore'); await render(); })));
        const streams = await play<{ id: number; name: string; volume: number }[]>('audio.streams');
        for (const stream of streams) { const [el, value] = field(`${stream.name} · stream ${stream.id}`, String(stream.volume), 'range'); value.min = '0'; value.max = '150'; value.onchange = () => void act(async () => { await play('audio.volume', { stream: stream.id, percent: Number(value.value) }); }); content.append(panel(el)); }
      } else if (page === 'history') {
        const history = await rpc<RunRecord[]>('history'); const analysis = chosen ? await rpc<{ summary: string; suggestions: string[] }>('analyze') : undefined;
        content.append(panel(title('Frame history'), text(analysis?.summary || 'Choose a game to analyze its recorded runs.'), ...(analysis?.suggestions || []).map(text), button('Ask Mind about this trace', () => bridge.call('mind.open', { text: `Analyze these local game measurements. Explain uncertainty and suggest a controlled comparison: ${JSON.stringify(analysis)}`, ask: true }))));
        for (const run of history) { const card = panel(h('strong', {}, `${games.find(g => g.id === run.game)?.name || run.game} · ${stamp(run.started)}`)); if (run.stats) { card.append(text(`${run.stats.avg_fps} FPS average · ${run.stats.p99_ms} ms p99 · ${run.stats.stutters} slow samples`)); const svg = document.createElementNS('http://www.w3.org/2000/svg', 'svg'); svg.setAttribute('viewBox', '0 0 600 90'); const line = document.createElementNS(svg.namespaceURI, 'polyline'); const max = Math.max(1, ...run.stats.points); line.setAttribute('points', run.stats.points.map((p, i, a) => `${i / Math.max(1, a.length - 1) * 600},${85 - p / max * 80}`).join(' ')); line.setAttribute('fill', 'none'); line.setAttribute('stroke', 'currentColor'); line.setAttribute('stroke-width', '2'); svg.append(line); card.append(svg); } else card.append(text('No MangoHud recording was found for this run.')); content.append(card); }
      } else {
        const events = await play<{ time: number; action: string; game?: string }[]>('activity'); content.append(panel(title('While you were away'), text('Recorded local game, storage and save activity.')));
        for (const event of events) content.append(panel(text(stamp(event.time)), h('strong', {}, event.action.replaceAll('-', ' ')), text(games.find(g => g.id === event.game)?.name || event.game || '')));
        const boot = await play<{ summary: string; services: string[] }>('boot'); content.append(panel(title('Boot trace'), text(boot.summary), ...boot.services.map(s => h('code', {}, s)))); if (!events.length) content.append(text('No recorded gaming activity yet.'));
      }
    } catch (e) { content.append(panel(title('Needs attention'), text(bridge.reason(e)), button('Storage settings', async () => { page = 'connections'; await render(); }), button('Retry', render))); }
    if (token === generation && root.isConnected) body.replaceChildren(content);
  }
  function updateGames(): void { games = [...scanned, ...nativeGames()]; select.replaceChildren(...games.map(g => h('option', { value: g.id }, `${g.name} · ${sourceLabel[g.source] || g.source}`))); chosen = games.some(g => g.id === chosen) ? chosen : games[0]?.id || ''; if (!games.length) select.append(h('option', { value: '' }, 'No installed games')); select.disabled = !games.length; select.value = chosen; }
  const offApps = store.on('apps', () => { if (!root.isConnected) { offApps(); return; } const before = games.map(g => g.id).join('\n'); updateGames(); if (before !== games.map(g => g.id).join('\n')) void render(); });
  window.addEventListener('pagehide', offApps, { once: true });
  void (async () => { try { scanned = (await gameCommand<GameLibrary>('scan')).games; } catch (e) { status.textContent = String(e); } updateGames(); await render(); })();
  return () => { generation++; offApps(); window.removeEventListener('pagehide', offApps); };
}
