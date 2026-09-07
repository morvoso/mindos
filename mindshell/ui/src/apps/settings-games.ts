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
  if (!r.ok) throw new Error(r.stderr.trim() || r.stdout.trim() || `mindos-dlss exited ${r.status}`);
  return r.json as T;
}

const SOURCE_LABEL: Record<string, string> = { steam: 'Steam', heroic: 'Heroic', lutris: 'Lutris', dir: 'Folder' };

export function gamesPage(el: HTMLElement, root: HTMLElement): () => void {
  const note = notice();
  const fail = (e: unknown) => {
    const msg = e instanceof Error ? e.message : String(e);
    if (/No such file or directory/.test(msg)) {
      note.show('mindos-dlss is part of the mindos-gaming package, which is not installed. Install it with: sudo pacman -S mindos-gaming', 'error');
      return;
    }
    note.show(msg, 'error');
  };
  let games: DlssGame[] = [];
  let library: DlssLibraryEntry[] = [];
  let kinds: DlssKind[] = [];
  let busy = false;

  const scanBtn = h('button', { class: 'btn' }, icon('refresh', 14), 'Scan again');
  const gamesBody = h('div', { class: 'dlss-games' });
  const gamesCard = card('Your games', h('p', { class: 'card-help' }, 'Games from Steam, Heroic and Lutris that include an upscaler. Replace the game’s DLL with a newer version for improved image quality or frame generation fixes. The original is kept and can be restored. Steam may reinstall the game’s own version after a verification or update.'), gamesBody, h('div', { class: 'card-actions' }, scanBtn));

  const libBody = h('div', { class: 'list' });
  const kindSel = h('select', { class: 'select' }) as HTMLSelectElement;
  const getBtn = h('button', { class: 'btn accent' }, icon('download', 14), 'Get a version…');
  const libCard = card('DLL library', h('p', { class: 'card-help' }, 'Versions downloaded from NVIDIA, AMD and Intel, and the versions included with the graphics driver. Games are updated from this library.'), libBody, h('div', { class: 'card-actions' }, kindSel, getBtn));

  el.append(pageHeader('Games', 'DLSS, FSR and XeSS versions per game, and the upscaler libraries available on this system.'), note.el, gamesCard, libCard);

  const withBusy = <T,>(p: Promise<T>): Promise<T> => {
    busy = true;
    el.classList.add('busy');
    return p.finally(() => {
      busy = false;
      el.classList.remove('busy');
    });
  };

  const load = async (rescan = false) => {
    try {
      const [g, l, k] = await withBusy(Promise.all([dlss<DlssGame[]>(rescan ? 'games' : 'scan'), dlss<DlssLibraryEntry[]>('library'), kinds.length ? Promise.resolve(kinds) : dlss<DlssKind[]>('kinds')]));
      games = g;
      library = l;
      kinds = k;
      render();
    } catch (e) {
      fail(e);
      render();
    }
  };

  const swap = (game: DlssGame, kind: string, version: string) => {
    withBusy(dlss(...['swap', game.id, kind, version]))
      .then(() => note.show(`${game.name}: ${kindLabel(kind)} ${version} swapped in.`, 'ok'))
      .catch(fail)
      .finally(() => void load());
  };
  const restore = (game: DlssGame, kind: string) => {
    withBusy(dlss('restore', game.id, kind))
      .then(() => note.show(`${game.name}: original ${kindLabel(kind)} restored.`, 'ok'))
      .catch(fail)
      .finally(() => void load());
  };
  const download = (kind: string, version: string) => {
    withBusy(dlss('download', kind, version))
      .then(() => note.show(`${kindLabel(kind)} ${version} added to the library.`, 'ok'))
      .catch(fail)
      .finally(() => void load());
  };
  const remove = (e: DlssLibraryEntry) => {
    withBusy(dlss('delete', e.kind, e.version))
      .then(() => note.show(`${e.label} ${e.version} deleted.`, 'ok'))
      .catch(fail)
      .finally(() => void load());
  };

  const kindLabel = (k: string) => kinds.find((x) => x.kind === k)?.label ?? k;
  const versionsFor = (kind: string) => library.filter((e) => e.kind === kind).map((e) => e.version).sort(compareVersions).reverse();

  const pickVersion = (kind: string) => {
    const body = h('div', { class: 'stack' }, h('div', { class: 'row-help' }, 'Fetching the list…'));
    const close = dialog(root, `${kindLabel(kind).toUpperCase()} VERSIONS`, body, [h('button', { class: 'btn', onclick: () => close() }, 'Close')]);
    dlss<DlssVersion[]>('versions', kind)
      .then((list) => {
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
      .catch((e) => body.replaceChildren(h('div', { class: 'row-help danger' }, `Could not fetch the list: ${e instanceof Error ? e.message : e}`)));
  };

  const render = () => {
    gamesBody.replaceChildren();
    if (!games.length) {
      gamesBody.appendChild(h('div', { class: 'row-help' }, busy ? 'Scanning…' : 'No games with an upscaler found. Install a game that uses DLSS, FSR 3.1 or XeSS and scan again.'));
    }
    for (const g of games) {
      const dlls = h('div', { class: 'dlss-dlls' });
      for (const d of g.dlls) {
        const versions = versionsFor(d.kind).filter((v) => v !== d.version);
        const sel = versions.length ? selectBox(versions.map((v) => ({ value: v, label: v })), versions[0], () => undefined) : null;
        const swapBtn = h('button', { class: 'btn small accent', disabled: !sel, title: sel ? '' : 'No newer version in the library', onclick: () => sel && swap(g, d.kind, sel.value) }, icon('swap', 12), 'Swap');
        const restoreBtn = d.swapped ? h('button', { class: 'btn small', onclick: () => restore(g, d.kind) }, icon('history', 12), 'Restore') : null;
        dlls.appendChild(
          h('div', { class: 'dlss-dll' }, h('span', { class: 'dlss-kind' }, d.label), h('span', { class: 'dlss-ver mono' }, d.version || '?'), d.swapped ? pill(`was ${d.backup_version ?? '?'}`, 'accent') : h('span'), h('span', { class: 'strip-gap' }), sel, swapBtn, restoreBtn),
        );
      }
      gamesBody.appendChild(h('div', { class: 'dlss-game' }, h('div', { class: 'dlss-game-head' }, h('span', { class: 'dlss-game-name' }, g.name), pill(SOURCE_LABEL[g.source] ?? g.source), h('span', { class: 'dlss-game-path mono' }, g.path)), dlls));
    }
    libBody.replaceChildren();
    if (!library.length) libBody.appendChild(h('div', { class: 'row-help' }, 'The library is empty. Download a version below. The driver’s own DLLs are listed here when nvidia-utils is installed.'));
    for (const e of [...library].sort((a, b) => a.kind.localeCompare(b.kind) || compareVersions(b.version, a.version))) {
      libBody.appendChild(row(`${e.label} ${e.version}`, `${e.source} · ${fmtBytes(e.size)}`, e.source === 'driver' ? pill('Driver', '') : h('button', { class: 'btn small danger', onclick: () => remove(e) }, icon('trash', 12), 'Delete')));
    }
    if (kindSel.options.length !== kinds.length) {
      kindSel.replaceChildren(...kinds.map((k) => h('option', { value: k.kind }, k.label)));
    }
  };

  scanBtn.addEventListener('click', () => void load(true));
  getBtn.addEventListener('click', () => pickVersion(kindSel.value || 'dlss'));
  render();
  void load();
  return () => undefined;
}

export function compareVersions(a: string, b: string): number {
  const pa = a.split('.').map(Number);
  const pb = b.split('.').map(Number);
  for (let i = 0; i < Math.max(pa.length, pb.length); i++) {
    const d = (pa[i] ?? 0) - (pb[i] ?? 0);
    if (d) return d;
  }
  return 0;
}
