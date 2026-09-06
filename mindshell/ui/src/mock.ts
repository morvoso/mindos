// A fake mindshell host for developing the UI in a browser. Installed by
// main.ts when window.webkit.messageHandlers.mindos is missing. It answers
// every bridge method with plausible data, keeps state (layout, windows,
// audio, edit mode) and emits the same events the real host would.

import { parseWindowInfo, setApi, type MindosGlobal } from './bridge';
import { letterIcon, hashHue } from './icons';
import { defaultLayout } from './layout';
import * as mfs from './mock-fs';
import type { AppInfo, AudioState, CatalogEntry, Layout, MenuItem, ModelEntry, ModelsInfo, Prefs, ShellState, Stats, TrayItem, WindowInfo, WmOutput } from './types';

type Listener = (payload: unknown) => void;

export interface MockHooks {
  openPopup?: (name: string, arg: unknown, keyboard: boolean) => void;
  closePopup?: (name: string) => void;
  /** A fit-to-content panel reported its length. */
  panelFit?: (panel: string, length: number) => void;
  openApp?: (name: string, page?: string, arg?: string) => void;
}

const GIB = 1073741824;

function mode(width: number, height: number, refresh: number, preferred = false, current = false) {
  return { width, height, refresh, preferred, current };
}

const OUTPUTS: WmOutput[] = [
  {
    name: 'DP-1', make: 'ASUS', model: 'PG32UCDM', x: 0, y: 0, width: 3840, height: 2160, scale: 1.5, refresh: 240, transform: 'normal',
    modes: [mode(3840, 2160, 240000, true, true), mode(3840, 2160, 144000), mode(3840, 2160, 120000), mode(3840, 2160, 60000), mode(2560, 1440, 240000), mode(2560, 1440, 120000), mode(1920, 1080, 240000), mode(1920, 1080, 60000)],
    enabled: true, vrr: true, vrr_supported: true, primary: true, mm_width: 697, mm_height: 392,
  },
  {
    name: 'HDMI-A-1', make: 'LG', model: '27GL850', x: 3840, y: 360, width: 2560, height: 1440, scale: 1, refresh: 144, transform: 'normal',
    modes: [mode(2560, 1440, 144000, true, true), mode(2560, 1440, 120000), mode(2560, 1440, 60000), mode(1920, 1080, 144000), mode(1920, 1080, 60000)],
    enabled: true, vrr: false, vrr_supported: true, primary: false, mm_width: 597, mm_height: 336,
  },
];

const MODELS_DIR = `${mfs.HOME}/mindos/models`;

function catalogEntry(id: string, name: string, file: string, size: number, params: string, description: string, min_vram_gb: number, recommended = false): CatalogEntry {
  return { id, name, file, url: `https://huggingface.co/unsloth/${id}/resolve/main/${file}`, size, license: 'Apache-2.0', license_url: 'https://huggingface.co/unsloth/' + id, params, description, min_vram_gb, recommended, installed: false };
}

const CATALOG: CatalogEntry[] = [
  catalogEntry('Qwen3.5-0.8B-GGUF', 'Qwen3.5 0.8B', 'Qwen3.5-0.8B-Q8_0.gguf', 0.9 * GIB, '0.8B · Q8_0', 'Tiny and instant. Fine for launching apps and simple questions; runs on anything.', 2),
  catalogEntry('Qwen3.5-4B-GGUF', 'Qwen3.5 4B', 'Qwen3.5-4B-Q4_K_M.gguf', 2.6 * GIB, '4B · Q4_K_M', 'The default: quick, good at following instructions and using tools. Needs about 4 GB of video memory.', 4, true),
  catalogEntry('Qwen3.5-9B-GGUF', 'Qwen3.5 9B', 'Qwen3.5-9B-Q4_K_M.gguf', 5.6 * GIB, '9B · Q4_K_M', 'Noticeably smarter answers and better code; still fast on a mid-range GPU.', 8),
  catalogEntry('Qwen3.5-27B-GGUF', 'Qwen3.5 27B', 'Qwen3.5-27B-Q4_K_M.gguf', 16.5 * GIB, '27B · Q4_K_M', 'The big one. Best answers, needs a 24 GB card to stay fully on the GPU.', 20),
];

export const mockHooks: MockHooks = {};

function app(id: string, name: string, categories: string[], opts: Partial<AppInfo> = {}): AppInfo {
  return {
    id: id.endsWith('.desktop') ? id : id + '.desktop',
    name,
    comment: opts.comment ?? '',
    exec: opts.exec ?? id.replace(/\.desktop$/, ''),
    icon: letterIcon(name, hashHue(name)),
    categories,
    terminal: opts.terminal ?? false,
  };
}

const APPS: AppInfo[] = [
  app('steam', 'Steam', ['Game'], { comment: 'Games and game library' }),
  app('lutris', 'Lutris', ['Game'], { comment: 'Open gaming platform' }),
  app('heroic', 'Heroic Games Launcher', ['Game'], { comment: 'Epic, GOG and Amazon games' }),
  app('org.prismlauncher.PrismLauncher', 'Prism Launcher', ['Game'], { comment: 'Minecraft launcher' }),
  app('firefox', 'Firefox', ['Network', 'WebBrowser'], { comment: 'Browse the web' }),
  app('discord', 'Discord', ['Network', 'Chat'], { comment: 'Voice, video and text chat' }),
  app('thunderbird', 'Thunderbird', ['Network', 'Email'], { comment: 'Mail and calendar' }),
  app('foot', 'foot', ['System', 'TerminalEmulator'], { comment: 'Wayland terminal' }),
  app('code', 'Visual Studio Code', ['Development', 'IDE'], { comment: 'Code editing, redefined' }),
  app('nvim', 'Neovim', ['Development', 'TextEditor'], { comment: 'Hyperextensible Vim', terminal: true }),
  app('org.kde.kate', 'Kate', ['Development', 'TextEditor'], { comment: 'Advanced text editor' }),
  app('gimp', 'GIMP', ['Graphics'], { comment: 'Image editor' }),
  app('blender', 'Blender', ['Graphics', '3DGraphics'], { comment: '3D creation suite' }),
  app('org.kde.krita', 'Krita', ['Graphics'], { comment: 'Digital painting' }),
  app('vlc', 'VLC', ['AudioVideo', 'Player'], { comment: 'Media player' }),
  app('spotify', 'Spotify', ['Audio', 'Music'], { comment: 'Music for everyone' }),
  app('com.obsproject.Studio', 'OBS Studio', ['AudioVideo', 'Recorder'], { comment: 'Streaming and recording' }),
  app('libreoffice-writer', 'LibreOffice Writer', ['Office', 'WordProcessor'], { comment: 'Word processor' }),
  app('org.kde.dolphin', 'Dolphin', ['System', 'FileManager'], { comment: 'File manager' }),
  app('htop', 'htop', ['System', 'Monitor'], { comment: 'Process viewer', terminal: true }),
  app('nvtop', 'nvtop', ['System', 'Monitor'], { comment: 'GPU process monitor', terminal: true }),
  app('org.gnome.Settings', 'Settings', ['Settings'], { comment: 'System settings' }),
  app('blueman-manager', 'Bluetooth', ['Settings'], { comment: 'Bluetooth devices' }),
  app('org.gnome.Calculator', 'Calculator', ['Utility', 'Calculator'], { comment: 'Do the maths' }),
  app('org.kde.ark', 'Ark', ['Utility', 'Archiving'], { comment: 'Archive manager' }),
  app('mangohud', 'MangoHud Config', ['Game', 'Settings'], { comment: 'Performance overlay' }),
  app('wine-Programs-7-Zip-7-Zip File Manager', '7-Zip File Manager', ['Utility', 'Archiving'], {
    comment: 'Archive manager (Windows)',
    exec: 'env WINEPREFIX=/home/mind/.wine wine C:\\\\ProgramData\\\\7-Zip.lnk',
    wmClass: '7zfm.exe',
    wine: true,
  }),
];

function win(id: number, title: string, app_id: string, extra: Partial<WindowInfo> = {}): WindowInfo {
  return { id, title, app_id, focused: false, fullscreen: false, maximized: true, minimized: false, x11: false, wine: false, output: 'Virtual-1', ...extra };
}

function trayIcon(letter: string, hue: number): string {
  const svg = `<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 24 24'><circle cx='12' cy='12' r='10' fill='hsl(${hue} 60% 45%)'/><text x='12' y='16.5' font-family='Inter,sans-serif' font-weight='700' font-size='13' text-anchor='middle' fill='white'>${letter}</text></svg>`;
  return 'data:image/svg+xml;charset=utf-8,' + encodeURIComponent(svg);
}

const TRAY: TrayItem[] = [
  { id: 'steam', title: 'Steam', tooltip: 'Steam — 2 friends online', icon: trayIcon('S', 210), status: 'active', hasMenu: true },
  { id: 'discord', title: 'Discord', tooltip: 'Discord', icon: trayIcon('D', 235), status: 'active', hasMenu: true },
  { id: 'nm-applet', title: 'Network', tooltip: 'Connected to Nebula-5G', icon: trayIcon('N', 160), status: 'passive', hasMenu: true },
];

const TRAY_MENU: MenuItem[] = [
  { id: 1, label: 'Library', enabled: true, type: 'item' },
  { id: 2, label: 'Friends', enabled: true, type: 'submenu', children: [
    { id: 21, label: 'Online', enabled: true, type: 'item', toggle: 'radio', checked: true },
    { id: 22, label: 'Away', enabled: true, type: 'item', toggle: 'radio', checked: false },
    { id: 23, label: 'Invisible', enabled: true, type: 'item', toggle: 'radio', checked: false },
  ] },
  { id: 3, label: 'Big Picture Mode', enabled: true, type: 'item' },
  { id: 4, label: '', enabled: true, type: 'separator' },
  { id: 5, label: 'Notifications', enabled: true, type: 'item', toggle: 'checkmark', checked: true },
  { id: 6, label: 'Settings', enabled: true, type: 'item' },
  { id: 7, label: '', enabled: true, type: 'separator' },
  { id: 8, label: 'Exit', enabled: true, type: 'item' },
];

export function installMock(): MindosGlobal {
  const q = new URLSearchParams(location.search);
  const listeners = new Map<string, Set<Listener>>();
  const emit = (event: string, payload: unknown) => {
    listeners.get(event)?.forEach((cb) => cb(payload));
  };
  let layout: Layout = defaultLayout();
  let editMode = q.get('edit') === '1';
  const windows: WindowInfo[] = [
    win(1, 'Steam', 'steam', { x11: true }),
    win(2, 'MindOS — Mozilla Firefox', 'firefox', { focused: true }),
    win(3, '~ — foot', 'foot'),
    win(4, 'C:\\Program Files\\7-Zip\\', '7zfm.exe', { x11: true, wine: true, maximized: false }),
    win(4, 'Discord', 'discord', { x11: true }),
    win(5, 'mindshell — Visual Studio Code', 'code'),
    win(6, 'Lutris', 'lutris', { minimized: true }),
  ];
  const audio: AudioState = { volume: 0.62, muted: false, sink: 'Starship/Matisse HD Audio' };
  const mind = { connected: true, ready: false, model: 'Qwen3.5-4B-Q4_K_M.gguf' };
  const popups = new Set<string>();
  let nextWin = 7;
  const t0 = Date.now();
  const info = parseWindowInfo();
  const appArg = (info.arg && typeof info.arg === 'object' ? info.arg : {}) as { page?: string; arg?: string };

  // ----- compositor state: layout mode, prefs, outputs -----
  const MODE_LIST = [{ name: 'floating', label: 'Floating' }, { name: 'dwindle', label: 'Tiles' }, { name: 'columns', label: 'Columns' }];
  let layoutMode = q.get('mode') ?? 'floating';
  const prefs: Prefs = { layout_mode: layoutMode, mind_show_tools: false, primary_output: null, outputs: {} };
  const outputs: WmOutput[] = JSON.parse(JSON.stringify(OUTPUTS));
  const modeEvent = () => ({ mode: layoutMode, label: MODE_LIST.find((m) => m.name === layoutMode)?.label ?? layoutMode, modes: MODE_LIST });
  const setMode = (m: string) => {
    if (!MODE_LIST.some((x) => x.name === m)) throw new Error(`unknown layout mode ${m}`);
    layoutMode = m;
    prefs.layout_mode = m;
    emit('layout_mode', modeEvent());
    emit('prefs', { prefs: { ...prefs } });
    return modeEvent();
  };
  const setOutput = (p: Record<string, unknown>) => {
    const o = outputs.find((x) => x.name === p.name);
    if (!o) throw new Error(`no output ${String(p.name)}`);
    const m = p.mode as { width: number; height: number; refresh: number } | undefined;
    if (m) {
      const found = o.modes.find((x) => x.width === m.width && x.height === m.height && x.refresh === m.refresh);
      if (!found) throw new Error('no such mode');
      for (const x of o.modes) x.current = x === found;
      o.width = m.width;
      o.height = m.height;
      o.refresh = m.refresh / 1000;
    }
    if (typeof p.scale === 'number') o.scale = p.scale;
    if (Array.isArray(p.position)) {
      o.x = Number(p.position[0]) || 0;
      o.y = Number(p.position[1]) || 0;
    }
    if (typeof p.transform === 'string') o.transform = p.transform;
    if (typeof p.enabled === 'boolean') o.enabled = p.enabled;
    if (typeof p.vrr === 'boolean') o.vrr = p.vrr && o.vrr_supported;
    if (p.primary === true) for (const x of outputs) x.primary = x === o;
    emit('outputs', { outputs: outputs.map((x) => ({ name: x.name, make: x.make, model: x.model, x: x.x, y: x.y, width: x.width, height: x.height, scale: x.scale, refresh: x.refresh })) });
    return { outputs: JSON.parse(JSON.stringify(outputs)) };
  };

  // ----- Mind (mindd) state -----
  const installed: ModelEntry[] = [
    { file: 'Qwen3.5-4B-Q4_K_M.gguf', path: `${MODELS_DIR}/Qwen3.5-4B-Q4_K_M.gguf`, size: 2.6 * GIB, active: true },
    { file: 'Qwen3.5-0.8B-Q8_0.gguf', path: `${MODELS_DIR}/Qwen3.5-0.8B-Q8_0.gguf`, size: 0.9 * GIB, active: false },
  ];
  const models: ModelsInfo = {
    type: 'models', current: installed[0].path, model: installed[0].file, ready: false, auto: true, thinking: false, models_dir: MODELS_DIR, external: false,
    gpu_memory: 24 * GIB, models: installed, catalog: CATALOG.map((c) => ({ ...c, installed: installed.some((m) => m.file === c.file) })), download: null,
  };
  let dlTimer: ReturnType<typeof setInterval> | undefined;
  const modelsReply = (): ModelsInfo => {
    models.catalog = models.catalog.map((c) => ({ ...c, installed: models.models.some((m) => m.file === c.file) }));
    models.models = models.models.map((m) => ({ ...m, active: m.path === models.current }));
    models.model = models.current ? models.current.split('/').pop()! : '';
    return JSON.parse(JSON.stringify(models));
  };
  const mindRequest = (req: Record<string, unknown>): unknown => {
    switch (req.type) {
      case 'models':
        return modelsReply();
      case 'set_model': {
        const path = String(req.path ?? '');
        if (!path || path === 'auto') {
          models.auto = true;
          models.current = installed[0].path;
        } else {
          models.auto = false;
          models.current = path;
          if (!models.models.some((m) => m.path === path)) models.models.push({ file: path.split('/').pop()!, path, size: 0, active: true });
        }
        models.ready = false;
        setTimeout(() => (models.ready = true), 2500);
        return modelsReply();
      }
      case 'set_thinking':
        models.thinking = !!req.enabled;
        return modelsReply();
      case 'download_model': {
        const file = String(req.file);
        const total = Number(req.size) || 3 * GIB;
        models.download = { file, url: String(req.url), received: 0, total, done: false, error: null };
        if (dlTimer) clearInterval(dlTimer);
        dlTimer = setInterval(() => {
          const d = models.download!;
          d.received = Math.min(total, d.received + total / 40);
          if (d.received >= total) {
            clearInterval(dlTimer);
            d.done = true;
            models.models.push({ file, path: `${MODELS_DIR}/${file}`, size: total, active: false });
            if (req.use_after) mindRequest({ type: 'set_model', path: `${MODELS_DIR}/${file}` });
          }
        }, 500);
        return modelsReply();
      }
      case 'cancel_download':
        if (dlTimer) clearInterval(dlTimer);
        if (models.download && !models.download.done) models.download = { ...models.download, done: true, error: 'cancelled' };
        return null;
      default:
        throw new Error(`unknown request ${String(req.type)}`);
    }
  };

  const stats = (): Stats => {
    const t = (Date.now() - t0) / 1000;
    const cpu = 18 + 12 * Math.sin(t / 3) + 6 * Math.sin(t * 1.7) + Math.random() * 4;
    const gpu = 42 + 30 * Math.sin(t / 5) + Math.random() * 5;
    return {
      cpu: Math.max(1, Math.min(100, cpu)),
      memUsed: 11.4 * 1073741824 + Math.sin(t / 7) * 0.4 * 1073741824,
      memTotal: 60 * 1073741824,
      gpu: { util: Math.max(0, Math.min(100, gpu)), temp: 58 + Math.sin(t / 9) * 6, mem: 6.2 * 1073741824, memTotal: 24 * 1073741824, name: 'RTX 4090' },
      load: [1.2, 0.9, 0.7],
      uptime: 3600 * 5 + 812 + t,
    };
  };

  const focused = () => windows.find((w) => w.focused)?.id ?? null;
  const pushWindows = () => emit('windows', { windows: windows.map((w) => ({ ...w })), focused: focused() });

  setTimeout(() => {
    mind.ready = true;
    emit('mind', { ...mind });
  }, 1500);

  const methods: Record<string, (p: Record<string, unknown>) => unknown> = {
    'shell.state': (): ShellState => ({
      user: 'morvoso',
      host: 'mindos-dev',
      uptime: 3600 * 5 + 812,
      outputs: [{ name: 'Virtual-1', make: 'QEMU', model: 'Virtual', x: 0, y: 0, width: 1920, height: 1080, scale: 1, refresh: 60000 }],
      windows: windows.map((w) => ({ ...w })),
      focused: focused(),
      apps: APPS,
      tray: TRAY,
      layout,
      editMode,
      config: { icon_theme: 'breeze-dark', hardware_acceleration: 'always', terminal: 'foot', icon_size: 48 },
      mind: { ...mind },
      audio: { ...audio },
      app: info.kind === 'app' ? { name: info.id, page: appArg.page, arg: appArg.arg } : null,
      version: '0.2.0 (mock)',
    }),
    'shell.ready': () => ({}),
    'shell.setEditMode': (p) => {
      editMode = !!p.enabled;
      emit('edit_mode', { enabled: editMode });
      return {};
    },
    'shell.exec': (p) => {
      console.info('[mock] exec', p.cmd);
      return {};
    },
    'shell.reload': () => {
      location.reload();
      return {};
    },
    'layout.get': () => layout,
    'layout.save': (p) => {
      layout = p.layout as Layout;
      emit('layout', { layout });
      return {};
    },
    'layout.reset': () => {
      layout = defaultLayout();
      emit('layout', { layout });
      return {};
    },
    'popup.open': (p) => {
      const name = String(p.name);
      popups.add(name);
      mockHooks.openPopup?.(name, p.arg, !!p.keyboard);
      emit('popup_state', { name, open: true, output: 'Virtual-1' });
      return {};
    },
    'popup.close': (p) => {
      const name = String(p.name ?? parseWindowInfo().popup ?? '');
      popups.delete(name);
      mockHooks.closePopup?.(name);
      emit('popup_state', { name, open: false, output: 'Virtual-1' });
      return {};
    },
    'popup.toggle': (p) => {
      const name = String(p.name);
      return popups.has(name) ? methods['popup.close'](p) : methods['popup.open'](p);
    },
    'windows.focus': (p) => {
      for (const w of windows) {
        w.focused = w.id === p.id;
        if (w.focused) w.minimized = false;
      }
      pushWindows();
      return {};
    },
    'windows.close': (p) => {
      const i = windows.findIndex((w) => w.id === p.id);
      if (i >= 0) {
        const wasFocused = windows[i].focused;
        windows.splice(i, 1);
        if (wasFocused && windows.length) windows[windows.length - 1].focused = true;
      }
      pushWindows();
      return {};
    },
    'windows.minimize': (p) => {
      const w = windows.find((x) => x.id === p.id);
      if (w) {
        w.minimized = true;
        w.focused = false;
      }
      pushWindows();
      return {};
    },
    'windows.toggleMinimize': (p) => {
      const w = windows.find((x) => x.id === p.id);
      if (w) {
        w.minimized = !w.minimized;
        if (!w.minimized) for (const o of windows) o.focused = o === w;
        else w.focused = false;
      }
      pushWindows();
      return {};
    },
    'apps.list': () => APPS,
    'apps.launch': (p) => {
      const a = APPS.find((x) => x.id === p.id);
      const name = a?.name ?? String(p.exec ?? 'app');
      setTimeout(() => {
        for (const o of windows) o.focused = false;
        windows.push(win(nextWin++, name, a ? a.id.replace(/\.desktop$/, '') : name.toLowerCase(), { focused: true }));
        pushWindows();
      }, 400);
      return {};
    },
    'tray.items': () => TRAY,
    'tray.activate': (p) => {
      console.info('[mock] tray activate', p.id);
      return {};
    },
    'tray.secondaryActivate': () => ({}),
    'tray.scroll': () => ({}),
    'tray.menu': () => TRAY_MENU,
    'tray.menuClick': (p) => {
      console.info('[mock] tray menu click', p.id, p.item);
      return {};
    },
    'mind.toggle': () => {
      console.info('[mock] mind bar toggled');
      return {};
    },
    'mind.status': () => ({ ...mind }),
    'system.power': (p) => {
      console.info('[mock] power', p.action);
      return {};
    },
    'system.stats': stats,
    'audio.get': () => ({ ...audio }),
    'audio.set': (p) => {
      audio.volume = Math.max(0, Math.min(1.5, Number(p.volume)));
      emit('audio', { ...audio });
      return {};
    },
    'audio.toggleMute': () => {
      audio.muted = !audio.muted;
      emit('audio', { ...audio });
      return {};
    },
    'network.status': () => ({ connected: true, kind: 'wifi', ssid: 'Nebula-5G', iface: 'wlan0', ip: '192.168.122.40' }),
    'battery.status': () => (q.get('battery') === '1' ? { present: true, percent: 67, charging: false, timeToEmpty: 8200 } : { present: false }),
    'icons.resolve': (p) => letterIcon(String(p.name), hashHue(String(p.name))),
    'panel.fit': (p) => {
      mockHooks.panelFit?.(String(p.panel ?? ''), Number(p.length) || 0);
      return {};
    },
    'wm.layoutMode': () => modeEvent(),
    'wm.setLayoutMode': (p) => setMode(String(p.mode)),
    'wm.cycleLayoutMode': () => setMode(MODE_LIST[(MODE_LIST.findIndex((m) => m.name === layoutMode) + 1) % MODE_LIST.length].name),
    'wm.outputs': () => ({ outputs: JSON.parse(JSON.stringify(outputs)) }),
    'wm.setOutput': (p) => setOutput(p),
    'prefs.get': () => ({ prefs: { ...prefs } }),
    'prefs.set': (p) => {
      Object.assign(prefs, (p.prefs as Prefs) ?? {});
      if (typeof prefs.layout_mode === 'string' && prefs.layout_mode !== layoutMode) setMode(prefs.layout_mode);
      emit('prefs', { prefs: { ...prefs } });
      return { prefs: { ...prefs } };
    },
    'mind.request': (p) => mindRequest((p.request as Record<string, unknown>) ?? {}),
    'wallpaper.list': () => mfs.wallpapers(),
    'fs.home': () => ({ path: mfs.HOME }),
    'fs.places': () => mfs.places(),
    'fs.list': (p) => mfs.list(String(p.path ?? mfs.HOME), !!p.hidden),
    'fs.stat': (p) => mfs.stat(String(p.path)),
    'fs.mkdir': (p) => mfs.mkdir(String(p.path), String(p.name)),
    'fs.rename': (p) => mfs.rename(String(p.path), String(p.name)),
    'fs.trash': (p) => mfs.remove((p.paths as string[]) ?? []),
    'fs.copy': (p) => mfs.transfer((p.paths as string[]) ?? [], String(p.dest), false),
    'fs.move': (p) => mfs.transfer((p.paths as string[]) ?? [], String(p.dest), true),
    'fs.open': (p) => {
      console.info('[mock] open', p.path);
      return {};
    },
    'shell.openApp': (p) => {
      console.info('[mock] open app', p.name, p.page, p.arg);
      mockHooks.openApp?.(String(p.name), p.page as string | undefined, p.arg as string | undefined);
      return {};
    },
    'app.close': () => {
      console.info('[mock] app close');
      return {};
    },
    'app.setTitle': (p) => {
      document.title = String(p.title ?? 'mindshell');
      return {};
    },
  };

  const m: MindosGlobal = {
    window: parseWindowInfo(),
    call(method, params) {
      const fn = methods[method];
      if (!fn) return Promise.reject(new Error(`mock: unknown method ${method}`));
      try {
        const result = fn(params ?? {});
        return new Promise((resolve) => setTimeout(() => resolve(result), 5));
      } catch (e) {
        return Promise.reject(e instanceof Error ? e : new Error(String(e)));
      }
    },
    on(event, cb) {
      let set = listeners.get(event);
      if (!set) listeners.set(event, (set = new Set()));
      set.add(cb);
      return () => set!.delete(cb);
    },
    _dispatch: emit,
  };
  setApi(m);
  return m;
}
