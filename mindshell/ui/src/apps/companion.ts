import { h } from '../dom';
import * as bridge from '../bridge';
import { gameCommand, nativeGames, type Game, type GameLibrary, runningWindow } from '../games';
import { play, type GameMeta } from '../gaming';
import { store } from '../state';

export function renderCompanion(root: HTMLElement, initialGame = ''): void {
  let game = initialGame, ducked: number | undefined, duckBusy = false, disposed = false;
  let scanned: Game[] = [];
  const status = h('p', { class: 'play-status', role: 'status' });
  const games = h('select', { 'aria-label': 'Companion game' });
  const outputs = h('select', { 'aria-label': 'Snap display' });
  const windows = h('select', { 'aria-label': 'Window to place beside companion' });
  const videoUrl = h('input', { class: 'input', placeholder: 'Local video path or direct HTTPS video URL', 'aria-label': 'Video source' });
  const video = h('video', { controls: true, preload: 'metadata', class: 'companion-video' });
  const wiki = h('input', { class: 'input', placeholder: 'https://…', 'aria-label': 'Wiki or guide URL' });
  const notes = h('textarea', { class: 'input companion-notes', placeholder: 'Boss patterns, checkpoints, build notes…', 'aria-label': 'Game notes' });
  const streams = h('select', { 'aria-label': 'Audio stream to duck' }, h('option', { value: '' }, 'No audio ducking'));
  const btn = (name: string, fn: () => Promise<unknown>) => h('button', { class: 'btn', onclick: async () => { try { await fn(); } catch (e) { status.textContent = String(e); } } }, name);
  async function restore(): Promise<void> { const old = ducked; ducked = undefined; if (old !== undefined) await play('audio.duck', { stream: old, enabled: false }); }
  async function duck(): Promise<void> {
    if (duckBusy || disposed) return;
    duckBusy = true;
    try {
      if (video.paused || video.ended || !streams.value) await restore();
      else { const id = Number(streams.value); if (ducked !== id) await restore(); ducked = id; await play('audio.duck', { stream: id, enabled: true, percent: 40 }); }
    } catch (e) { status.textContent = String(e); }
    finally { duckBusy = false; }
  }
  video.onplay = () => void duck(); video.onpause = () => void duck(); video.onended = () => void duck();
  video.onerror = () => { status.textContent = 'Video could not be played. Use a local video or direct media URL; open streaming sites in the browser.'; void restore(); };
  streams.onchange = () => void duck();
  const pulse = setInterval(() => { if (!root.isConnected) { dispose(); return; } if (!video.paused || ducked !== undefined) void duck(); }, 3000);
  const off = store.on('windows', listWindows);
  const offOutputs = store.on('outputs', listWindows);
  function dispose(): void { if (disposed) return; disposed = true; clearInterval(pulse); video.pause(); void restore(); off(); offOutputs(); offApps(); window.removeEventListener('pagehide', dispose); }
  window.addEventListener('pagehide', dispose);
  function listWindows(): void {
    const selected = windows.value, display = outputs.value;
    windows.replaceChildren(h('option', { value: '' }, 'Choose game or browser window'), ...store.state.windows.filter(w => !/Game Companion/.test(w.title)).map(w => h('option', { value: String(w.id) }, w.title || w.app_id)));
    if (store.state.windows.some(w => String(w.id) === selected)) windows.value = selected;
    outputs.replaceChildren(...store.state.outputs.map(o => h('option', { value: o.name }, `${o.name} · ${o.width}×${o.height}`)));
    if (store.state.outputs.some(o => o.name === display)) outputs.value = display;
  }
  async function snap(release = false): Promise<void> {
    const own = await bridge.call<{ id: number } | null>('windows.current');
    if (!own) throw new Error('The compositor has not registered this companion window yet.');
    const id = Number(windows.value);
    if (!release && !id) throw new Error('Choose the game or browser window to place beside the companion.');
    if (id) await bridge.call('windows.snap', { id, zone: release ? 'release' : 'left-two-thirds', output: outputs.value });
    await bridge.call('windows.snap', { id: own.id, zone: release ? 'release' : 'right-third', output: outputs.value });
    status.textContent = release ? 'Window positions restored.' : 'Pinned to the right third. The selected window occupies the left two thirds.';
  }
  async function load(): Promise<void> {
    await restore(); video.pause(); video.removeAttribute('src'); video.load();
    if (!game) return;
    const meta = await play<GameMeta>('metadata.get', { game });
    videoUrl.value = meta.video || ''; wiki.value = meta.wiki || ''; notes.value = meta.notes || '';
  }
  games.onchange = () => { game = games.value; void load().catch(e => { status.textContent = String(e); }); };
  root.append(h('div', { class: 'gaming-app companion-app' }, h('header', { class: 'play-header' }, h('h1', {}, 'Companion'), games), status,
    h('section', { class: 'play-card' }, h('span', { class: 'game-eyebrow' }, 'PINNED / RIGHT THIRD'), windows, outputs, h('div', { class: 'play-row' }, btn('Pin beside game', () => snap()), btn('Restore windows', () => snap(true)))),
    video, h('section', { class: 'play-card' }, videoUrl, h('div', { class: 'play-row' }, btn('Load video', async () => {
      const raw = videoUrl.value.trim(); const url = raw.startsWith('/') ? new URL(`file://${raw.split('/').map(encodeURIComponent).join('/')}`) : new URL(raw);
      if (!['https:', 'file:'].includes(url.protocol)) throw new Error('Use a local file or HTTPS media URL.');
      video.src = url.protocol === 'file:' && location.protocol === 'mindos:' ? (await bridge.call<{ uri: string }>('media.open', { path: decodeURIComponent(url.pathname) })).uri : url.href;
      video.load(); status.textContent = 'Video loaded. Press play to start.';
    }), btn('Picture in picture', async () => { const v = video as HTMLVideoElement & { webkitSetPresentationMode?: (mode: string) => void }; if (document.pictureInPictureEnabled) await video.requestPictureInPicture(); else if (v.webkitSetPresentationMode) v.webkitSetPresentationMode('picture-in-picture'); else throw new Error('Picture in picture is unavailable in this WebKit build. Use the pinned companion window.'); })),
      h('label', { class: 'play-field' }, 'Lower selected app audio by 40% during playback', streams), btn('Refresh audio streams', async () => { const list = await play<{ id: number; name: string }[]>('audio.streams'); streams.replaceChildren(h('option', { value: '' }, 'No audio ducking'), ...list.map(s => h('option', { value: s.id }, s.name))); await restore(); })),
    h('section', { class: 'play-card' }, h('h2', {}, 'Guide & notes'), wiki, btn('Open guide in browser', async () => { const uri = new URL(wiki.value); if (!['https:', 'http:'].includes(uri.protocol)) throw new Error('Use an HTTP or HTTPS guide URL.'); await bridge.call('shell.open', { uri: uri.href }); }), notes, btn('Save game companion', async () => { if (!game) throw new Error('Choose a game first.'); await play('metadata.set', { game, settings: { video: videoUrl.value, wiki: wiki.value, notes: notes.value } }); status.textContent = 'Guide, video and notes saved for this game.'; })),
    h('section', { class: 'play-card' }, h('h2', {}, 'Mind'), btn('Ask Mind about this game', () => bridge.call('mind.open', { text: `Help with ${games.selectedOptions[0]?.textContent || game}. My notes: ${notes.value}` })))));
  listWindows();
  function updateGames(): void {
    const list = [...scanned, ...nativeGames()];
    games.replaceChildren(...list.map(g => h('option', { value: g.id }, g.name)));
    if (!list.some(g => g.id === game)) game = list[0]?.id || '';
    if (!list.length) games.append(h('option', { value: '' }, 'No installed games'));
    games.disabled = !list.length;
    games.value = game;
    const g = list.find(g => g.id === game); const w = g && runningWindow(g);
    if (w) windows.value = String(w.id);
  }
  const offApps = store.on('apps', () => {
    const previous = game; updateGames();
    if (game !== previous) void load().catch(e => { status.textContent = String(e); });
  });
  void gameCommand<GameLibrary>('scan').catch(e => { status.textContent = String(e); return { games: [], warnings: [] } as GameLibrary; }).then(async library => {
    if (disposed) return;
    scanned = library.games;
    updateGames();
    await load();
  }).catch(e => { status.textContent = String(e); });
}
