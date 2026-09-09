// Settings › Games: the DLSS / FSR / XeSS swapper (mindos-dlss). Every
// game the launchers know about, the upscaler DLLs it ships with, and the
// versions on hand to swap in.

import * as bridge from '../bridge';
import { h } from '../dom';
import { icon } from '../icons';
import type { DlssGame, DlssKind, DlssLibraryEntry, DlssVersion, RunResult } from '../types';
import { card, dialog, fmtBytes, notice, pageHeader, pill, row, selectBox } from './shared';

async function dlss<T>(...args: string[]): Promise<T> {
  const r = await bridge.call<RunResult>('shell.run', { argv: ['mindos-dlss', '--json', ...args] });
  if (!r.ok) throw new Error((r.json as { error?: string } | undefined)?.error || r.stderr.trim() || r.stdout.trim() || `mindos-dlss exited ${r.status}`);
  if (r.json == null) throw new Error('Game tools returned an unreadable response. Update mindos-gaming and try again.');
  return r.json as T;
}

const SOURCE_LABEL: Record<string, string> = { steam: 'Steam', heroic: 'Heroic', lutris: 'Lutris', dir: 'Folder' };

// A settings-page change must not launch a second package transaction or lose
// the running operation. The native helper keeps running while pages change.
let installing: Promise<RunResult> | undefined;
const missingHelper = (msg: string) => /^mindos-dlss: No such file or directory(?: \(os error 2\))?$/.test(msg);

export function gamesPage(el: HTMLElement, root: HTMLElement): () => void {
  const note = notice();
  const fail = (e: unknown) => {
    const msg = bridge.reason(e);
    if (missingHelper(msg)) {
      missing = true;
      loaded = false;
      note.clear();
      return;
    }
    note.show(msg, 'error');
  };
  let games: DlssGame[] = [];
  let library: DlssLibraryEntry[] = [];
  let kinds: DlssKind[] = [];
  let busy = false;
  let loaded = false;
  let alive = true;
  let missing = false;
  let settingUp = false;
  let versionDialog: (() => void) | undefined;

  const scanBtn = h('button', { class: 'btn', 'aria-label': 'Scan installed games again' }, icon('refresh', 14), 'Scan again');
  const search = h('input', { class: 'input dlss-search', type: 'search', placeholder: 'Filter games…', 'aria-label': 'Filter games by name or launcher' });
  const count = h('span', { class: 'row-help', role: 'status', 'aria-live': 'polite' });
  const gamesBody = h('div', { class: 'dlss-games' });
  const gamesCard = card('Your games', h('p', { class: 'card-help' }, 'Tune DLSS, FSR and XeSS in your installed games. Close the game before applying a version. The original is kept for Restore; after a game update, reapplying uses its updated DLL as the new original and keeps the older backup.'), h('div', { class: 'dlss-toolbar' }, search, count, scanBtn), gamesBody);

  const libBody = h('div', { class: 'list' });
  const kindSel = h('select', { class: 'select' }) as HTMLSelectElement;
  const getBtn = h('button', { class: 'btn accent' }, icon('download', 14), 'Get a version…');
  const libCard = card('DLL library', h('p', { class: 'card-help' }, 'Downloaded and imported versions, plus the versions included with the graphics driver. Choose a version for each game; a higher version number does not guarantee better results.'), libBody, h('div', { class: 'card-actions' }, kindSel, getBtn));

  const installBtn = h('button', { class: 'btn accent' }, icon('download', 14), 'Install gaming tools');
  const checkBtn = h('button', { class: 'btn', onclick: () => void load() }, icon('refresh', 14), 'Check again');
  const setupCard = card('Get ready to play',
    h('p', { class: 'card-help' }, 'Add Steam, Lutris and Wine for your games, GameMode and MangoHud for tuning, with optional apps available in Software. The bundle also includes DLSS, FSR and XeSS version management.'),
    h('div', { class: 'card-actions' }, installBtn, checkBtn),
    h('p', { class: 'row-help' }, 'Requires an internet connection and administrator authentication. Downloads may take several minutes. Pending system updates may be installed with the bundle.'));
  setupCard.classList.add('games-setup');
  const guideBtn = h('button', { class: 'btn', onclick: async () => {
    try { await bridge.call('shell.exec', { cmd: 'xdg-open /usr/share/doc/mindos/GAMES.md' }); }
    catch (e) { if (alive) fail(e); }
  } }, icon('external', 14), 'Gaming guide');
  el.append(pageHeader('Games', 'Set up your gaming tools and tune DLSS, FSR and XeSS for each game.'), note.el, setupCard, gamesCard, libCard, h('div', { class: 'card-actions' }, guideBtn));

  // Native disabled controls cover keyboard activation too. Keep each
  // command and its following rescan in one operation, including error paths.
  const updateControls = () => {
    el.setAttribute('aria-busy', String(busy));
    el.classList.toggle('busy', busy);
    el.querySelectorAll<HTMLButtonElement | HTMLSelectElement>('button, select').forEach((control) => {
      control.disabled = busy || control.dataset.unavailable === 'true';
    });
  };
  const fetchState = async () => {
    const [g, l, k] = await Promise.all([dlss<DlssGame[]>('scan'), dlss<DlssLibraryEntry[]>('library'), kinds.length ? Promise.resolve(kinds) : dlss<DlssKind[]>('kinds')]);
    if (!alive) return;
    games = g;
    library = l;
    kinds = k;
    loaded = true;
    missing = false;
  };
  const install = async () => {
    if (busy || !alive) return;
    busy = settingUp = missing = true;
    render();
    note.show('Installing gaming tools… Authenticate when prompted. You can visit other settings while this finishes.', 'info', 0);
    if (!installing) {
      const job = bridge.call<RunResult>('shell.run', { argv: ['pkexec', 'mindos-pkg', 'install', '--repo-only', 'mindos-gaming'] });
      installing = job;
      const finished = () => { if (installing === job) installing = undefined; };
      void job.then(finished, finished);
    }
    try {
      const result = await installing;
      if (!alive) return;
      if (!result.ok) {
        if (result.status === 126) note.show('Installation cancelled. You can try again when you are ready.', 'info');
        else note.show(`Could not install gaming tools: ${result.stderr.trim() || result.stdout.trim() || `exit ${result.status}`}`, 'error');
        return;
      }
      try {
        await fetchState();
        if (alive) note.show('Gaming tools installed. Open Steam or Lutris from the launcher to install a game.', 'ok');
      } catch (e) {
        if (alive) note.show(`Installation finished, but the game scan is unavailable: ${bridge.reason(e)}. Use Check again to retry.`, 'error');
      }
    } catch (e) {
      if (alive) note.show(`Could not install gaming tools: ${bridge.reason(e)}`, 'error');
    } finally {
      busy = settingUp = false;
      if (alive) {
        render();
        if (!missing) scanBtn.focus();
      }
    }
  };
  const load = async () => {
    if (busy || !alive) return;
    busy = true;
    updateControls();
    count.textContent = 'Scanning…';
    try {
      await fetchState();
      if (alive) note.clear();
    } catch (e) {
      if (alive) fail(e);
    } finally {
      busy = false;
      if (alive) render();
    }
  };
  const change = async (args: string[], progress: string, success: string) => {
    if (busy || !alive) return;
    busy = true;
    updateControls();
    note.show(progress, 'info', 0);
    let outcome = success;
    let failed = false;
    try {
      await dlss(...args);
    } catch (e) {
      failed = true;
      outcome = bridge.reason(e);
    }
    try {
      // Also refresh after failures: multi-DLL operations may have completed
      // some files, and the helper records each one before replacing it.
      await fetchState();
    } catch (e) {
      failed = true;
      outcome += ` Could not refresh: ${bridge.reason(e)}`;
    } finally {
      busy = false;
      if (alive) {
        render();
        note.show(outcome, failed ? 'error' : 'ok');
      }
    }
  };
  const swap = (game: DlssGame, kind: string, version: string) =>
    void change(['swap', game.id, kind, version], `Applying ${kindLabel(kind)} to ${game.name}…`, `${game.name}: ${kindLabel(kind)} ${version} applied.`);
  const restore = (game: DlssGame, kind: string) =>
    void change(['restore', game.id, kind], `Restoring ${game.name}…`, `${game.name}: original ${kindLabel(kind)} restored.`);
  const download = (kind: string, version: string) =>
    void change(['download', kind, version], `Downloading ${kindLabel(kind)} ${version}…`, `${kindLabel(kind)} ${version} added to the library.`);
  const remove = (entry: DlssLibraryEntry) =>
    void change(['delete', entry.kind, entry.version], `Removing ${entry.label} ${entry.version}…`, `${entry.label} ${entry.version} removed from the library.`);

  const kindLabel = (k: string) => kinds.find((x) => x.kind === k)?.label ?? k;
  const versionsFor = (kind: string) => [...new Set(library.filter((e) => e.kind === kind).map((e) => e.version))].sort(compareVersions).reverse();

  const pickVersion = (kind: string) => {
    if (busy || !alive || root.querySelector('.sheet-backdrop')) return;
    const body = h('div', { class: 'stack' }, h('div', { class: 'row-help' }, 'Fetching the list…'));
    const closeDialog = dialog(root, `${kindLabel(kind).toUpperCase()} VERSIONS`, body, [h('button', { class: 'btn', onclick: () => close() }, 'Close')]);
    const close = () => { closed = true; versionDialog = undefined; closeDialog(); };
    let closed = false;
    versionDialog = close;
    dlss<DlssVersion[]>('versions', kind)
      .then((list) => {
        if (!alive || closed || !body.isConnected) return;
        body.replaceChildren();
        const rows = h('div', { class: 'list dlss-versions' });
        for (const v of list.slice(0, 40)) {
          rows.appendChild(
            row(`${v.version}${v.label ? ' · ' + v.label : ''}${v.dev ? ' · dev' : ''}`, `${v.signed || ''}${v.size ? ' · ' + fmtBytes(v.size) : ''}${v.description ? ' · ' + v.description : ''}`, v.installed ? pill('In library', 'ok') : h('button', { class: 'btn small accent', onclick: () => { close(); download(kind, v.version); } }, icon('download', 12), 'Get')),
          );
        }
        if (!list.length) rows.appendChild(h('div', { class: 'row-help' }, 'No versions listed.'));
        body.appendChild(rows);
      })
      .catch((e) => { if (alive && !closed && body.isConnected) body.replaceChildren(h('div', { class: 'row-help danger' }, `Could not fetch the list: ${bridge.reason(e)}`)); });
  };

  const render = () => {
    setupCard.hidden = !missing;
    gamesCard.hidden = libCard.hidden = missing;
    installBtn.replaceChildren(icon('download', 14), settingUp ? 'Installing…' : 'Install gaming tools');
    gamesBody.replaceChildren();
    const query = search.value.trim().toLocaleLowerCase();
    const visible = games.filter((g) => `${g.name} ${SOURCE_LABEL[g.source] ?? g.source}`.toLocaleLowerCase().includes(query));
    count.textContent = busy ? 'Scanning…' : loaded ? `${visible.length} of ${games.length} ${games.length === 1 ? 'game' : 'games'}` : 'Scan unavailable';
    if (!visible.length) {
      gamesBody.appendChild(h('div', { class: 'row-help' }, !loaded ? 'Scan your games to see available upscalers. Use Scan again to retry.' : games.length ? 'No games match this filter.' : 'No games with an upscaler found. Games using DLSS, FSR 3.1 or XeSS appear here after installation.'));
    }
    for (const g of visible) {
      const dlls = h('div', { class: 'dlss-dlls' });
      for (const d of g.dlls) {
        const versions = versionsFor(d.kind).filter((v) => v !== d.version || d.changed);
        const sel = versions.length ? selectBox(versions.map((v) => ({ value: v, label: v })), versions[0], () => undefined) : null;
        sel?.setAttribute('aria-label', `${g.name}: ${d.label} version`);
        const swapBtn = h('button', { class: 'btn small accent', dataset: { unavailable: String(!sel) }, 'aria-label': `Apply ${d.label} to ${g.name}`, title: sel ? '' : 'Download another version to apply it', onclick: () => sel && swap(g, d.kind, sel.value) }, icon('swap', 12), d.changed ? 'Reapply' : 'Apply');
        const restoreBtn = (d.restorable ?? d.swapped) ? h('button', { class: 'btn small', 'aria-label': `Restore original ${d.label} for ${g.name}`, onclick: () => restore(g, d.kind) }, icon('history', 12), 'Restore') : null;
        dlls.appendChild(
          h('div', { class: 'dlss-dll' }, h('div', { class: 'dlss-info' }, h('span', { class: 'dlss-kind' }, d.label), h('span', { class: 'dlss-ver mono' }, d.version || '?'), d.changed ? pill('Game DLL changed', 'warn') : d.swapped ? pill(`Original ${d.backup_version ?? '?'}`, 'accent') : null), h('div', { class: 'dlss-actions' }, sel, swapBtn, restoreBtn)),
        );
      }
      gamesBody.appendChild(h('div', { class: 'dlss-game' }, h('div', { class: 'dlss-game-head' }, h('span', { class: 'dlss-game-name' }, g.name), pill(SOURCE_LABEL[g.source] ?? g.source), h('span', { class: 'dlss-game-path mono', title: g.path }, g.path)), dlls));
    }
    libBody.replaceChildren();
    if (loaded && !library.length) libBody.appendChild(h('div', { class: 'row-help' }, 'The library is empty. Download a version below. The driver’s own DLLs are listed here when nvidia-utils is installed.'));
    for (const e of [...library].sort((a, b) => a.kind.localeCompare(b.kind) || compareVersions(b.version, a.version))) {
      libBody.appendChild(row(`${e.label} ${e.version}`, `${e.source} · ${fmtBytes(e.size)}`, e.source === 'driver' ? pill('Driver', '') : h('button', { class: 'btn small danger', onclick: () => remove(e) }, icon('trash', 12), 'Delete')));
    }
    if (kindSel.options.length !== kinds.length) {
      kindSel.replaceChildren(...kinds.map((k) => h('option', { value: k.kind }, k.label)));
      kindSel.setAttribute('aria-label', 'Upscaler library to download');
    }
    kindSel.dataset.unavailable = String(!kinds.length);
    getBtn.dataset.unavailable = String(!kinds.length);
    updateControls();
  };

  search.addEventListener('input', render);
  scanBtn.addEventListener('click', () => void load());
  getBtn.addEventListener('click', () => pickVersion(kindSel.value || 'dlss'));
  installBtn.addEventListener('click', () => void install());
  render();
  if (installing) void install();
  else void load();
  return () => { alive = false; versionDialog?.(); el.classList.remove('busy'); el.removeAttribute('aria-busy'); };
}

export function compareVersions(a: string, b: string): number {
  const pa = a.split('.').map((p) => /^\d+$/.test(p) ? Number(p) : -1);
  const pb = b.split('.').map((p) => /^\d+$/.test(p) ? Number(p) : -1);
  for (let i = 0; i < Math.max(pa.length, pb.length); i++) {
    const d = (pa[i] ?? 0) - (pb[i] ?? 0);
    if (d) return d;
  }
  return 0;
}
