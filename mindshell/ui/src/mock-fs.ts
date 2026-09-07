// An in-memory file system for the browser mock: enough of a home folder to
// develop the Files app and the wallpaper picker without a host.

import type { FsEntry, FsListing, FsStat, Place, WallpaperEntry } from './types';

export interface Node {
  name: string;
  dir: boolean;
  size: number;
  mtime: number;
  mime: string;
  children?: Node[];
}

const NOW = Math.floor(Date.now() / 1000);
const day = 86400;

function file(name: string, size: number, age = 3, mime?: string): Node {
  const ext = name.includes('.') ? name.split('.').pop()!.toLowerCase() : '';
  const guess: Record<string, string> = {
    png: 'image/png', jpg: 'image/jpeg', jpeg: 'image/jpeg', webp: 'image/webp', md: 'text/markdown', txt: 'text/plain',
    pdf: 'application/pdf', iso: 'application/x-cd-image', mp4: 'video/mp4', mkv: 'video/x-matroska', ods: 'application/vnd.oasis.opendocument.spreadsheet',
    toml: 'application/toml', rs: 'text/rust', ts: 'text/x-typescript', json: 'application/json', zip: 'application/zip', gguf: 'application/octet-stream',
    sh: 'application/x-shellscript', desktop: 'application/x-desktop', flac: 'audio/flac', mp3: 'audio/mpeg',
  };
  return { name, dir: false, size, mtime: NOW - age * day, mime: mime ?? guess[ext] ?? 'application/octet-stream' };
}

function dir(name: string, children: Node[], age = 5): Node {
  return { name, dir: true, size: 0, mtime: NOW - age * day, mime: 'inode/directory', children };
}

export const HOME = '/home/morvoso';

const root: Node = dir('', [
  dir('bin', []),
  dir('boot', [file('vmlinuz-linux-mindos', 14_800_000, 12), file('initramfs-linux-mindos.img', 36_000_000, 12)]),
  dir('etc', [dir('mindos', [file('mind.toml', 1_200, 20), file('shell.toml', 640, 20), file('model-catalog.json', 3_400, 8)]), file('fstab', 512, 60), file('hostname', 11, 60)]),
  dir('home', [
    dir('morvoso', [
      dir('Desktop', [dir('Mods', []), file('firefox.desktop', 4_200, 30, 'application/x-desktop'), file('steam.desktop', 3_900, 30, 'application/x-desktop'), file('notes.md', 2_100, 1), file('screenshot.png', 610_000, 0), file('setup.sh', 1_800, 4)]),
      dir('Documents', [file('notes.md', 4_210, 1), file('budget.ods', 28_400, 9), file('thesis.pdf', 2_140_000, 40), dir('Scans', [file('passport.jpg', 1_900_000, 200)])]),
      dir('Downloads', [file('mindos-2026.09-x86_64.iso', 2_900_000_000, 0), file('Qwen3.5-9B-Q4_K_M.gguf', 5_700_000_000, 2), file('screenshot-2026-09-01.png', 388_000, 5), file('setup.sh', 3_100, 7)]),
      dir('Games', [dir('Cyberpunk 2077', []), dir('Hades II', []), dir('Factorio', []), file('saves.zip', 84_000_000, 3)]),
      dir('Music', [file('Pulsar.flac', 42_000_000, 30), file('Neon Drive.mp3', 9_800_000, 31)]),
      dir('Pictures', [
        dir('Wallpapers', [file('nebula.png', 4_100_000, 14), file('circuit.jpg', 2_300_000, 14), file('dunes.jpg', 3_050_000, 21), file('void.png', 1_200_000, 21), file('aurora.webp', 900_000, 2)]),
        file('cat.png', 1_400_000, 3),
        file('vacation.jpg', 3_900_000, 90),
      ]),
      dir('Projects', [dir('mindos', [file('README.md', 9_800, 0), file('Makefile', 4_300, 1), dir('mindwm', []), dir('mindshell', []), dir('mindd', [])]), dir('dotfiles', [])]),
      dir('Videos', [file('clip.mp4', 210_000_000, 4)]),
      dir('.config', [dir('mindos', [file('shell.toml', 300, 3), file('layout.json', 5_100, 0)])]),
      dir('.local', [dir('share', [])]),
      file('.bashrc', 3_900, 100),
      file('TODO.txt', 512, 0),
    ]),
  ]),
  dir('usr', [dir('share', [dir('backgrounds', [file('arch.png', 2_800_000, 300), file('gruvbox-mountains.jpg', 1_900_000, 300)]), dir('mindos', [file('wallpaper.png', 3_300_000, 30)]), dir('applications', [])])]),
  dir('var', [dir('log', [file('pacman.log', 900_000, 0)])]),
]);

const splitPath = (p: string) => p.split('/').filter(Boolean);

export function lookup(path: string): Node | undefined {
  let node: Node = root;
  for (const part of splitPath(path)) {
    const next = node.children?.find((c) => c.name === part);
    if (!next) return undefined;
    node = next;
  }
  return node;
}

function parentOf(path: string): { node: Node; name: string } | undefined {
  const parts = splitPath(path);
  const name = parts.pop();
  if (name === undefined) return undefined;
  const node = lookup('/' + parts.join('/'));
  return node && node.dir ? { node, name } : undefined;
}

// ----- icons and thumbnails ---------------------------------------------------

const svg = (body: string, vb = '0 0 48 48') => 'data:image/svg+xml;charset=utf-8,' + encodeURIComponent(`<svg xmlns='http://www.w3.org/2000/svg' viewBox='${vb}'>${body}</svg>`);
const ICON_FOLDER = svg(`<path d='M4 12a2 2 0 0 1 2-2h12l4 4h20a2 2 0 0 1 2 2v22a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2z' fill='#0e2c38' stroke='#19e3ff' stroke-width='1.5'/><path d='M4 20h40' stroke='#19e3ff' stroke-opacity='.5'/>`);
const ICON_FILE = svg(`<path d='M10 4h18l10 10v30H10z' fill='#161d26' stroke='#8b9bb0' stroke-width='1.5'/><path d='M28 4v10h10' fill='none' stroke='#8b9bb0' stroke-width='1.5'/>`);
const iconFor = (kind: string, color: string, glyph: string) => svg(`<path d='M10 4h18l10 10v30H10z' fill='#161d26' stroke='${color}' stroke-width='1.5'/><path d='M28 4v10h10' fill='none' stroke='${color}' stroke-width='1.5'/><text x='24' y='36' font-family='Inter,sans-serif' font-weight='700' font-size='11' text-anchor='middle' fill='${color}'>${glyph}</text>`) + `#${kind}`;

export function mockIcon(n: Node): string {
  if (n.dir) return ICON_FOLDER;
  const [kind, sub] = n.mime.split('/');
  if (kind === 'text' || sub === 'json' || sub === 'toml') return iconFor('text', '#8b9bb0', 'TXT');
  if (kind === 'video') return iconFor('video', '#a78bfa', 'VID');
  if (kind === 'audio') return iconFor('audio', '#a78bfa', 'AUD');
  if (sub === 'pdf') return iconFor('pdf', '#ff5d8f', 'PDF');
  if (sub === 'zip' || sub === 'x-cd-image') return iconFor('archive', '#ffb454', 'ZIP');
  if (sub === 'x-shellscript') return iconFor('sh', '#3ddc97', 'SH');
  return ICON_FILE;
}

function hash(s: string): number {
  let h = 0;
  for (let i = 0; i < s.length; i++) h = (h * 31 + s.charCodeAt(i)) >>> 0;
  return h;
}

/** A fake photo: a gradient with a couple of shapes, seeded by the name. */
export function mockThumb(name: string): string {
  const h1 = hash(name) % 360;
  const h2 = (h1 + 60 + (hash(name + 'x') % 120)) % 360;
  const body = `<defs><linearGradient id='g' x1='0' y1='0' x2='1' y2='1'><stop offset='0' stop-color='hsl(${h1} 60% 22%)'/><stop offset='1' stop-color='hsl(${h2} 70% 12%)'/></linearGradient></defs><rect width='320' height='180' fill='url(#g)'/><circle cx='${60 + (hash(name) % 200)}' cy='${40 + (hash(name + 'y') % 100)}' r='${30 + (hash(name + 'r') % 40)}' fill='hsl(${h2} 80% 60%)' fill-opacity='.35'/><path d='M0 150 L80 100 L150 140 L230 80 L320 130 V180 H0z' fill='hsl(${h1} 50% 8%)' fill-opacity='.7'/>`;
  return svg(body, '0 0 320 180');
}

// ----- the fs.* methods ---------------------------------------------------------

const isImage = (n: Node) => n.mime.startsWith('image/');

function entry(parent: string, n: Node): FsEntry {
  const path = `${parent === '/' ? '' : parent}/${n.name}`;
  return {
    name: n.name,
    path,
    dir: n.dir,
    size: n.size,
    mtime: n.mtime,
    hidden: n.name.startsWith('.'),
    symlink: false,
    mime: n.mime,
    icon: mockIcon(n),
    image: isImage(n),
    thumb: isImage(n) ? mockThumb(n.name) : undefined,
  };
}

export function normalize(path: string): string {
  const p = path.replace(/^~(?=\/|$)/, HOME);
  const parts: string[] = [];
  for (const seg of p.split('/')) {
    if (!seg || seg === '.') continue;
    if (seg === '..') parts.pop();
    else parts.push(seg);
  }
  return '/' + parts.join('/');
}

export function list(pathRaw: string, hidden: boolean): FsListing {
  const path = normalize(pathRaw || HOME);
  const n = lookup(path);
  if (!n) throw new Error(`No such folder: ${path}`);
  if (!n.dir) throw new Error(`Not a folder: ${path}`);
  const entries = (n.children ?? [])
    .filter((c) => hidden || !c.name.startsWith('.'))
    .map((c) => entry(path, c))
    .sort((a, b) => (a.dir !== b.dir ? (a.dir ? -1 : 1) : a.name.localeCompare(b.name, undefined, { numeric: true, sensitivity: 'base' })));
  const parent = path === '/' ? null : normalize(path + '/..');
  return { path, parent, entries };
}

export function stat(pathRaw: string): FsStat {
  const path = normalize(pathRaw);
  const n = lookup(path);
  if (!n) throw new Error(`No such file: ${path}`);
  return {
    path,
    name: n.name || '/',
    dir: n.dir,
    size: n.size,
    mtime: n.mtime,
    mime: n.mime,
    items: n.dir ? n.children?.length ?? 0 : undefined,
    permissions: n.dir ? 'rwxr-xr-x' : 'rw-r--r--',
    mode: n.dir ? 0o755 : 0o644,
  };
}

export function places(): Place[] {
  return [
    { name: 'Home', path: HOME, icon: 'home', kind: 'home' },
    { name: 'Desktop', path: `${HOME}/Desktop`, icon: 'desktop', kind: 'folder' },
    { name: 'Documents', path: `${HOME}/Documents`, icon: 'note', kind: 'folder' },
    { name: 'Downloads', path: `${HOME}/Downloads`, icon: 'download', kind: 'folder' },
    { name: 'Games', path: `${HOME}/Games`, icon: 'gamepad', kind: 'folder' },
    { name: 'Music', path: `${HOME}/Music`, icon: 'music', kind: 'folder' },
    { name: 'Pictures', path: `${HOME}/Pictures`, icon: 'image', kind: 'folder' },
    { name: 'Videos', path: `${HOME}/Videos`, icon: 'window', kind: 'folder' },
    { name: 'System', path: '/', icon: 'hdd', kind: 'system' },
    { name: 'Games SSD', path: '/run/media/morvoso/games', icon: 'hdd', kind: 'mount', removable: false },
    { name: 'USB stick', path: '/run/media/morvoso/USB', icon: 'usb', kind: 'mount', removable: true },
  ];
}

export function mkdir(parentRaw: string, name: string): { path: string } {
  const parent = normalize(parentRaw);
  const n = lookup(parent);
  if (!n?.dir) throw new Error(`Not a folder: ${parent}`);
  let final = name;
  let i = 2;
  while (n.children!.some((c) => c.name === final)) final = `${name} (${i++})`;
  n.children!.push(dir(final, [], 0));
  return { path: `${parent === '/' ? '' : parent}/${final}` };
}

export function rename(pathRaw: string, name: string): { path: string } {
  const path = normalize(pathRaw);
  const p = parentOf(path);
  const n = lookup(path);
  if (!p || !n) throw new Error(`No such file: ${path}`);
  if (name.includes('/') || !name || name === '.' || name === '..') throw new Error('Not a valid name');
  if (p.node.children!.some((c) => c.name === name)) throw new Error(`${name} already exists`);
  n.name = name;
  return { path: `${normalize(path + '/..') === '/' ? '' : normalize(path + '/..')}/${name}` };
}

export function remove(paths: string[]): { count: number } {
  let count = 0;
  for (const raw of paths) {
    const path = normalize(raw);
    const p = parentOf(path);
    if (!p) continue;
    const i = p.node.children!.findIndex((c) => c.name === p.name);
    if (i >= 0) {
      p.node.children!.splice(i, 1);
      count++;
    }
  }
  return { count };
}

export function transfer(paths: string[], destRaw: string, moving: boolean): { count: number } {
  const dest = normalize(destRaw);
  const d = lookup(dest);
  if (!d?.dir) throw new Error(`Not a folder: ${dest}`);
  let count = 0;
  for (const raw of paths) {
    const path = normalize(raw);
    const n = lookup(path);
    const p = parentOf(path);
    if (!n || !p) continue;
    if (d.children!.some((c) => c.name === n.name)) throw new Error(`${n.name} already exists in ${dest}`);
    const copy: Node = JSON.parse(JSON.stringify(n));
    d.children!.push(copy);
    if (moving) {
      const i = p.node.children!.indexOf(n);
      if (i >= 0) p.node.children!.splice(i, 1);
    }
    count++;
  }
  return { count };
}

export function wallpapers(): WallpaperEntry[] {
  const out: WallpaperEntry[] = [];
  const scan = (path: string, folder: string) => {
    const n = lookup(path);
    for (const c of n?.children ?? []) if (isImage(c)) out.push({ path: `${path}/${c.name}`, name: c.name.replace(/\.[^.]+$/, ''), folder, thumb: mockThumb(c.name) });
  };
  scan('/usr/share/mindos', 'MindOS');
  scan('/usr/share/backgrounds', 'System');
  scan(`${HOME}/Pictures/Wallpapers`, 'Wallpapers');
  scan(`${HOME}/Pictures`, 'Pictures');
  return out;
}
