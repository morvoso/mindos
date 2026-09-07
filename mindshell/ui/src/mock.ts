// A fake mindshell host for developing the UI in a browser. Installed by
// main.ts when window.webkit.messageHandlers.mindos is missing. It answers
// every bridge method with plausible data, keeps state (layout, windows,
// audio, edit mode) and emits the same events the real host would.

import { parseWindowInfo, setApi, type MindosGlobal } from './bridge';
import { letterIcon, hashHue } from './icons';
import { defaultLayout } from './layout';
import * as mfs from './mock-fs';
import type { AppInfo, AudioState, CatalogEntry, DlssGame, DlssLibraryEntry, HealthReport, Layout, MenuItem, MindNotice, ModelEntry, ModelsInfo, Notification, PerfStatus, PolkitRequest, Prefs, ShellState, Stats, TrayItem, UpdateStatus, WindowInfo, WmOutput } from './types';

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
  // An XEmbed icon hosted by the compositor (a Windows program under Wine): no menu protocol.
  { id: 'x11:4194305', title: 'Notepad++', tooltip: 'Notepad++', icon: trayIcon('N', 95), status: 'active', hasMenu: false, xembed: true },
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
  const now = () => Math.floor(Date.now() / 1000);
  // ----- notices, updates, health (the Mind daemon subscription) -----
  const notices: MindNotice[] = [
    { id: 'updates:available', level: 'warn', title: '14 updates, one of them the NVIDIA driver', body: 'nvidia-utils 580.65 → 580.82 and linux-mindos 6.17.3 → 6.17.4. The driver and kernel change together, so expect a reboot; nothing in the news mentions manual steps. A snapshot is taken first.', source: 'updates', time: now() - 1800, actions: [{ label: 'Update now', kind: 'request', arg: { type: 'apply_updates' } }, { label: 'What changes?', kind: 'chat', arg: 'What is in the pending update and could it break my games?' }, { label: 'Details', kind: 'settings', arg: 'updates' }] },
    { id: 'health:pacnew', level: 'info', title: 'A config file wants a look', body: '/etc/pacman.conf.pacnew arrived with the last update. Your file is untouched; the new one may have new defaults.', source: 'health', time: now() - 7200, actions: [{ label: 'Show the difference', kind: 'chat', arg: 'Show me what changed in /etc/pacman.conf.pacnew versus my /etc/pacman.conf' }] },
  ];
  let autoApply = false;
  const updates: UpdateStatus = {
    checked_at: now() - 1800,
    packages: [
      { name: 'linux-mindos', from: '6.17.3-1', to: '6.17.4-1', tag: 'kernel' }, { name: 'nvidia-utils', from: '580.65.06-2', to: '580.82.07-1', tag: 'gpu' }, { name: 'lib32-nvidia-utils', from: '580.65.06-2', to: '580.82.07-1', tag: 'gpu' },
      { name: 'mesa', from: '25.2.3-1', to: '25.2.4-1', tag: 'gpu' }, { name: 'steam', from: '1.0.0.82-1', to: '1.0.0.83-1', tag: 'gaming' }, { name: 'pipewire', from: '1.4.7-1', to: '1.4.8-1', tag: 'gaming' },
      { name: 'firefox', from: '143.0-1', to: '143.0.1-1', tag: '' }, { name: 'curl', from: '8.16.0-1', to: '8.16.0-2', tag: '' }, { name: 'gtk4', from: '4.20.1-1', to: '4.20.2-1', tag: 'graphics' },
      { name: 'mindshell', from: '0.2.0-15', to: '0.2.0-16', tag: 'mindos' }, { name: 'python', from: '3.13.7-1', to: '3.13.7-2', tag: '' }, { name: 'zsh', from: '5.9-6', to: '5.9-7', tag: '' }, { name: 'git', from: '2.51.0-1', to: '2.51.1-1', tag: '' }, { name: 'openssl', from: '3.5.2-1', to: '3.5.3-1', tag: 'core' },
    ],
    news: [{ title: 'linux-firmware split into several packages', date: '2026-08-21', url: 'https://archlinux.org/news/' }, { title: 'Manual intervention for pacman 7.1', date: '2026-07-02', url: 'https://archlinux.org/news/' }],
    risk: 'medium',
    summary: 'A routine driver and kernel bump. The NVIDIA release notes list fixes for frame pacing under Wayland and nothing that removes a feature; the kernel is a stable patch. Update when you are not about to play, and reboot after.',
    warnings: ['The NVIDIA driver and the kernel change together: reboot right after.', 'openssl updates; long-running programs keep the old library until restarted.'],
    manual_intervention: false, reboot: true, assessed_by_model: true, assessing: false, checking: false, applying: false, auto_apply: autoApply,
    last_update: { time: now() - 86400 * 3, packages: ['firefox', 'foot', 'mesa', 'lib32-mesa', 'vulkan-radeon', 'libx11', 'harfbuzz'], pre_snapshot: 42, ok: true, verified: 'ok', report: 'All services up, the kernel matches the running one, the GPU driver loaded, 412 GB free.' },
    error: '',
  };
  const health: HealthReport = { checked_at: now() - 600, findings: [
    { id: 'pacnew', level: 'info', title: '/etc/pacman.conf.pacnew', body: 'A new default config arrived with an update. Yours is untouched.', actions: [{ label: 'Show the difference', kind: 'chat', arg: 'Show me the difference between /etc/pacman.conf and /etc/pacman.conf.pacnew' }] },
    { id: 'failed-units', level: 'ok', title: 'No failed services', body: '', actions: [] },
    { id: 'kernel', level: 'ok', title: 'Kernel 6.17.3 is the installed one', body: '', actions: [] },
    { id: 'nvidia', level: 'ok', title: 'NVIDIA 580.65.06 loaded', body: '', actions: [] },
    { id: 'disk', level: 'ok', title: '412 GB free on /', body: '', actions: [] },
    { id: 'snapshots', level: 'ok', title: '12 boot snapshots', body: '', actions: [] },
  ] };
  const notifications: Notification[] = [
    { id: 1, app: 'Steam', desktop: 'steam', icon: APPS.find((a) => a.id === 'steam.desktop')?.icon ?? '', summary: 'Cyberpunk 2077 updated', body: 'Patch 2.3 installed (4.1 GB).', actions: [{ key: 'default', label: 'Open' }], urgency: 1, resident: false, transient: false, category: '', timeout: -1, time: now() - 300, replaced: false },
    { id: 2, app: 'Firefox', desktop: 'firefox', icon: APPS.find((a) => a.id === 'firefox.desktop')?.icon ?? '', summary: 'Download finished', body: 'proton-ge-10-12.tar.gz', actions: [{ key: 'default', label: 'Open' }, { key: 'show', label: 'Show in folder' }], urgency: 1, resident: false, transient: false, category: 'transfer.complete', timeout: -1, time: now() - 2400, replaced: false },
  ];
  let nextNotification = 3;
  let dnd = false;
  const notifyState = () => ({ items: notifications.map((n) => ({ ...n })), dnd });
  const pushNotify = (added?: Notification, closed?: number) => emit('notify', { ...notifyState(), added: added ?? null, closed: closed ?? null });
  const pushNotices = (added?: MindNotice) => emit('mind_notices', { notices: notices.map((n) => ({ ...n })), added: added ?? null });
  // ----- the authentication dialog (polkit) -----
  const polkit: PolkitRequest = { id: 1, action: 'org.freedesktop.systemd1.manage-units', message: 'Authentication is required to start "docker.service".', icon: '', user: 'morvoso', users: ['morvoso', 'root'], command: '/usr/bin/systemctl enable --now docker.service', error: '', attempt: 1, tries: 3, busy: false };
  const perf: PerfStatus = { mode: 'balanced', effective: 'balanced', game: 0, gameMode: 'performance', mindSleeps: true, cpu: 'AMD Ryzen 7 9800X3D 8-Core Processor', driver: 'amd-pstate-epp', governor: 'schedutil', epp: 'balance_performance', boost: true, platformProfile: 'balanced', thp: 'always', scheduler: 'EEVDF+BORE', scx: '', nvidia: false, gpu: 'NVIDIA GeForce RTX 4090', powerLimit: 'default' };
  const dlssGames: DlssGame[] = [
    { id: 'steam:1091500', name: 'Cyberpunk 2077', source: 'steam', path: `${mfs.HOME}/.local/share/Steam/steamapps/common/Cyberpunk 2077`, dlls: [{ kind: 'dlss', label: 'DLSS Super Resolution', file: 'bin/x64/nvngx_dlss.dll', version: '3.7.10.0', swapped: false }, { kind: 'dlss_g', label: 'DLSS Frame Generation', file: 'bin/x64/nvngx_dlssg.dll', version: '3.7.10.0', swapped: false }, { kind: 'dlss_d', label: 'DLSS Ray Reconstruction', file: 'bin/x64/nvngx_dlssd.dll', version: '3.7.10.0', swapped: false }] },
    { id: 'steam:2358720', name: 'Black Myth: Wukong', source: 'steam', path: `${mfs.HOME}/.local/share/Steam/steamapps/common/BlackMythWukong`, dlls: [{ kind: 'dlss', label: 'DLSS Super Resolution', file: 'b1/Binaries/Win64/nvngx_dlss.dll', version: '310.2.1.0', swapped: true, backup_version: '3.7.20.0' }, { kind: 'fsr_31_dx12', label: 'FSR 3.1 (DX12)', file: 'b1/Binaries/Win64/amd_fidelityfx_dx12.dll', version: '3.1.4.0', swapped: false }] },
    { id: 'heroic:alan-wake-2', name: 'Alan Wake 2', source: 'heroic', path: `${mfs.HOME}/Games/Heroic/AlanWake2`, dlls: [{ kind: 'dlss', label: 'DLSS Super Resolution', file: 'nvngx_dlss.dll', version: '3.5.10.0', swapped: false }, { kind: 'xess', label: 'XeSS', file: 'libxess.dll', version: '1.3.1.0', swapped: false }] },
  ];
  const dlssLibrary: DlssLibraryEntry[] = [
    { kind: 'dlss', label: 'DLSS Super Resolution', version: '310.2.1.0', path: `${mfs.HOME}/.local/share/mindos/dlss/dlss/310.2.1.0/nvngx_dlss.dll`, source: 'download', size: 45 * 1048576 },
    { kind: 'dlss_g', label: 'DLSS Frame Generation', version: '310.2.1.0', path: '/usr/lib/nvidia/wine/nvngx_dlssg.dll', source: 'driver', size: 28 * 1048576 },
    { kind: 'dlss', label: 'DLSS Super Resolution', version: '3.8.10.0', path: `${mfs.HOME}/.local/share/mindos/dlss/dlss/3.8.10.0/nvngx_dlss.dll`, source: 'download', size: 44 * 1048576 },
  ];
  const KINDS = [['dlss', 'nvngx_dlss.dll', 'DLSS Super Resolution'], ['dlss_d', 'nvngx_dlssd.dll', 'DLSS Ray Reconstruction'], ['dlss_g', 'nvngx_dlssg.dll', 'DLSS Frame Generation'], ['fsr_31_dx12', 'amd_fidelityfx_dx12.dll', 'FSR 3.1 (DX12)'], ['fsr_31_vk', 'amd_fidelityfx_vk.dll', 'FSR 3.1 (Vulkan)'], ['xess', 'libxess.dll', 'XeSS'], ['xess_fg', 'libxess_fg.dll', 'XeSS Frame Generation']].map(([kind, dll, label]) => ({ kind, dll, label }));
  const docker = { active: true, enabled: true, member: false };
  // pkexec: the host would hand this to polkit, so the mock opens the same
  // dialog and only runs the command once the password went through.
  let pkexecPending: { argv: string[]; done: (r: unknown) => void } | undefined;
  const askPolkit = (argv: string[]) =>
    new Promise((done) => {
      polkit.command = `/usr/bin/${argv.join(' ')}`;
      polkit.message = `Authentication is required to run "${argv[0]}".`;
      polkit.error = '';
      polkit.attempt = 1;
      polkit.busy = false;
      pkexecPending = { argv, done };
      emit('polkit', { ...polkit });
      mockHooks.openPopup?.('auth', {}, true);
    });
  const runHelper = (argv: string[]): unknown => {
    const ok = (json: unknown, stdout = '') => ({ status: 0, ok: true, stdout: stdout || JSON.stringify(json), stderr: '', json });
    const [a0, a1, a2, a3, a4] = argv[0] === 'sudo' ? argv.slice(2) : argv;
    if (a0 === 'mindos-perf') {
      if (a1 === 'status') return ok({ ...perf });
      if (a1 === 'set') {
        perf.mode = a2 as PerfStatus['mode'];
        if (!perf.game) perf.effective = perf.mode;
        perf.scx = perf.effective === 'performance' ? 'scx_lavd' : '';
        perf.governor = perf.effective === 'performance' ? 'performance' : perf.effective === 'quiet' ? 'powersave' : 'schedutil';
        perf.nvidia = perf.effective === 'performance';
        return ok(null, perf.game ? `mode ${a2} saved; ${perf.gameMode} stays in effect until the game ends` : `mode ${a2}`);
      }
      if (a1 === 'config') {
        if (a2 === 'GAME_MODE') perf.gameMode = a3 as PerfStatus['gameMode'];
        if (a2 === 'MIND_SLEEPS_WHILE_GAMING') perf.mindSleeps = a3 === '1';
        if (a2 === 'SCX_SCHEDULER') perf.scheduler = a3 || 'EEVDF+BORE';
        if (a2 === 'NVIDIA_POWER_LIMIT') perf.powerLimit = a3;
        return ok(null, `${a2}=${a3}`);
      }
    }
    if (a0 === 'mindos-dlss') {
      const cmd = a1 === '--json' ? a2 : a1;
      const rest = a1 === '--json' ? [a3, a4] : [a2, a3];
      if (cmd === 'scan' || cmd === 'games') return ok(dlssGames);
      if (cmd === 'library') return ok(dlssLibrary);
      if (cmd === 'kinds') return ok(KINDS);
      if (cmd === 'versions') return ok(['310.2.1.0', '310.1.0.0', '3.8.10.0', '3.7.20.0', '3.7.10.0', '3.5.10.0'].map((v, i) => ({ version: v, installed: dlssLibrary.some((e) => e.kind === rest[0] && e.version === v), label: i === 0 ? 'latest' : '', dev: false, signed: `2025-0${(i % 8) + 1}-1${i}`, size: 44 * 1048576, description: 'NVIDIA DLSS' })));
      if (cmd === 'download') {
        dlssLibrary.push({ kind: rest[0], label: KINDS.find((k) => k.kind === rest[0])?.label ?? rest[0], version: rest[1] === 'latest' ? '310.2.1.0' : rest[1], path: `${mfs.HOME}/.local/share/mindos/dlss/${rest[0]}/${rest[1]}/x.dll`, source: 'download', size: 44 * 1048576 });
        return ok({});
      }
      if (cmd === 'delete') {
        const i = dlssLibrary.findIndex((e) => e.kind === rest[0] && e.version === rest[1]);
        if (i >= 0) dlssLibrary.splice(i, 1);
        return ok({});
      }
      if (cmd === 'swap' || cmd === 'restore') {
        const g = dlssGames.find((x) => x.id === rest[0]);
        const d = g?.dlls.find((x) => x.kind === rest[1]);
        if (d) {
          if (cmd === 'swap') {
            if (!d.swapped) d.backup_version = d.version;
            d.version = argv[argv.length - 1];
            d.swapped = true;
          } else {
            d.version = d.backup_version ?? d.version;
            d.swapped = false;
            delete d.backup_version;
          }
        }
        return ok({});
      }
    }
    if (a0 === 'mindos-dev-setup') return ok({ tools: [['gcc', '15.2.1'], ['clang', '20.1.8'], ['rustc', '1.90.0'], ['go', '1.25.1'], ['node', '24.8.0'], ['python', '3.13.7'], ['uv', '0.8.17'], ['docker', '28.4.0'], ['podman', '5.6.1'], ['distrobox', '1.8.1.2'], ['git', '2.51.0'], ['lazygit', '0.55.0'], ['just', '1.43.0'], ['mold', '2.40.4'], ['sccache', '0.10.0'], ['perf', '6.17'], ['gdb', '16.3'], ['hyperfine', '1.19.0'], ['starship', '1.23.0'], ['zoxide', '0.9.8'], ['shellcheck', null]].map(([name, version]) => ({ name, version })), docker: { ...docker } });
    if (a0 === 'mindos-boot') return ok(null, `    #  date              kind    kernel        description
   44* 2026-09-07 09:12  single  6.17.3-1      boot
   43  2026-09-04 18:40  post    6.17.3-1      pacman -Syu
   42  2026-09-04 18:39  pre     6.17.3-1      pacman -Syu  [important]
   41  2026-09-01 11:02  single  6.17.2-1      before mindos-dlss swap
   40  2026-08-28 20:15  post    6.17.2-1      pacman -S mindos-dev
   39  2026-08-28 20:14  pre     6.17.2-1      pacman -S mindos-dev
* booted right now (changes stay in RAM); 'mindos-boot restore' makes it the system`);
    if (a0 === 'pkexec') return askPolkit(argv.slice(1));
    if (a0 === 'systemctl' && a1 === 'enable') {
      docker.active = true;
      docker.enabled = true;
      return ok(null, '');
    }
    if (a0 === 'usermod') {
      docker.member = true;
      return ok(null, '');
    }
    return { status: 127, ok: false, stdout: '', stderr: `mock: ${argv.join(' ')} not available`, json: null };
  };
  // A game "starts" after a while so the perf widget shows GameMode at work.
  setTimeout(() => {
    perf.game = 1;
    perf.effective = perf.gameMode || perf.mode;
    perf.scx = perf.effective === 'performance' ? 'scx_lavd' : '';
    perf.nvidia = perf.effective === 'performance';
    emit('mind', { ...mind, sleeping: true });
  }, 40000);
  const mind = { connected: true, ready: false, model: 'Qwen3.5-4B-Q4_K_M.gguf', daemon: true, sleeping: false, notices, updates, health };
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
      case 'notices':
        return { type: 'notices', notices: notices.map((n) => ({ ...n })) };
      case 'dismiss_notice': {
        const i = notices.findIndex((n) => n.id === req.id);
        if (i >= 0) notices.splice(i, 1);
        pushNotices();
        return { type: 'notices', notices: notices.map((n) => ({ ...n })) };
      }
      case 'updates':
        if (req.check) {
          updates.checking = true;
          emit('mind_updates', { ...updates });
          setTimeout(() => {
            updates.checking = false;
            updates.checked_at = now();
            emit('mind_updates', { ...updates });
          }, 1500);
        }
        return { type: 'updates', ...updates };
      case 'apply_updates':
        updates.applying = true;
        emit('mind_updates', { ...updates });
        setTimeout(() => {
          updates.last_update = { time: now(), packages: updates.packages.map((p) => p.name), pre_snapshot: 45, ok: true, verified: '', report: '' };
          updates.packages = [];
          updates.applying = false;
          updates.risk = '';
          updates.summary = '';
          updates.warnings = [];
          updates.reboot = false;
          emit('mind_updates', { ...updates });
          const i = notices.findIndex((n) => n.id === 'updates:available');
          if (i >= 0) notices.splice(i, 1);
          const done: MindNotice = { id: 'updates:reboot', level: 'warn', title: 'Updated; reboot when you can', body: 'The kernel and the NVIDIA driver changed. Until the reboot, new games may fail to start.', source: 'updates', time: now(), actions: [{ label: 'Reboot', kind: 'request', arg: { type: 'power', action: 'reboot' } }, { label: 'Later', kind: 'request', arg: { type: 'dismiss_notice', id: 'updates:reboot' } }] };
          notices.unshift(done);
          pushNotices(done);
        }, 4000);
        return { type: 'updates', ...updates };
      case 'set_auto_update':
        autoApply = !!req.enabled;
        updates.auto_apply = autoApply;
        emit('mind_updates', { ...updates });
        return { type: 'updates', ...updates };
      case 'health':
        health.checked_at = now();
        emit('mind_health', { ...health });
        return { type: 'health', ...health };
      case 'rollback':
        return { type: 'ok' };
      case 'power':
        return { type: 'ok' };
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
  setTimeout(() => {
    const n: Notification = { id: nextNotification++, app: 'Steam', desktop: 'steam', icon: APPS.find((a) => a.id === 'steam.desktop')?.icon ?? '', summary: 'A friend is online', body: 'morvoso started playing Hades II.', actions: [{ key: 'default', label: 'Open' }, { key: 'join', label: 'Join' }], urgency: 1, resident: false, transient: false, category: 'presence.online', timeout: -1, time: now(), replaced: false };
    notifications.push(n);
    pushNotify(n);
  }, 2500);

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
      notify: notifyState(),
      polkit: { ...polkit },
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
      // The host cancels the polkit request when its dialog goes away.
      if (name === 'auth' && pkexecPending) methods['polkit.cancel']({});
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
    // The login screen (kind=greeter): two accounts, "hunter2" signs in,
    // "otp" asks for a second factor.
    'greeter.info': () => ({
      users: [
        { name: 'morvoso', display: 'Justin', avatar: null },
        { name: 'guest', display: 'Guest', avatar: null },
      ],
      sessions: [
        { id: 'mindos', name: 'MindOS', exec: 'mindos-session' },
        { id: 'sway', name: 'Sway', exec: 'sway' },
      ],
      last: { user: 'morvoso', session: 'mindos' },
      host: 'mindos-dev',
    }),
    'greeter.login': (p) => {
      if (p.password === 'hunter2') return { status: 'started' };
      if (p.password === 'otp') return { status: 'prompt', secret: false, message: 'One-time code:', notes: ['Enter the code from your phone.'] };
      return { status: 'failed', message: 'Login failed', notes: [] };
    },
    'greeter.respond': (p) => (p.response === '123456' ? { status: 'started' } : { status: 'failed', message: 'Login failed', notes: ['That code was not accepted.'] }),
    'greeter.cancel': () => ({}),
    'greeter.done': () => {
      console.info('[mock] greeter done, the session would start now');
      return {};
    },
    'greeter.power': (p) => {
      console.info('[mock] greeter power', p.action);
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
    'mind.notices': () => ({ notices: notices.map((n) => ({ ...n })) }),
    'mind.dismiss': (p) => mindRequest({ type: 'dismiss_notice', id: p.id }),
    'mind.act': (p) => {
      const a = p.action as { kind: string; arg: unknown };
      if (a.kind === 'request') return mindRequest(a.arg as Record<string, unknown>);
      if (a.kind === 'settings') mockHooks.openApp?.('settings', String(a.arg ?? ''));
      if (a.kind === 'chat') console.log('mock: Mind bar would open with', a.arg);
      return null;
    },
    'notify.list': () => notifyState(),
    'notify.close': (p) => {
      const i = notifications.findIndex((n) => n.id === p.id);
      if (i >= 0) notifications.splice(i, 1);
      pushNotify(undefined, Number(p.id));
      return { closed: i >= 0 };
    },
    'notify.action': (p) => {
      console.log('mock: notification action', p.id, p.key);
      const i = notifications.findIndex((n) => n.id === p.id);
      if (i >= 0 && !notifications[i].resident) {
        notifications.splice(i, 1);
        pushNotify(undefined, Number(p.id));
      }
      return null;
    },
    'notify.clear': () => {
      notifications.splice(0);
      pushNotify();
      return null;
    },
    'notify.setDnd': (p) => {
      dnd = !!p.enabled;
      pushNotify();
      return { enabled: dnd };
    },
    'toast.fit': () => ({ visible: true }),
    'polkit.respond': (p: Record<string, unknown>) => {
      polkit.busy = true;
      emit('polkit', { ...polkit });
      setTimeout(() => {
        if (String(p.password) === 'mindos') {
          emit('polkit', null);
          mockHooks.closePopup?.('auth');
          const pending = pkexecPending;
          pkexecPending = undefined;
          pending?.done(runHelper(pending.argv));
          return;
        }
        polkit.busy = false;
        polkit.error = 'That password did not work.';
        polkit.attempt = Math.min(polkit.attempt + 1, polkit.tries);
        emit('polkit', { ...polkit });
      }, 700);
      return {};
    },
    'polkit.cancel': () => {
      emit('polkit', null);
      const pending = pkexecPending;
      pkexecPending = undefined;
      pending?.done({ status: 126, ok: false, stdout: '', stderr: '', json: null });
      return {};
    },
    'shell.run': (p) => runHelper((p.argv as string[]) ?? []),
    'wallpaper.list': () => mfs.wallpapers(),
    'fs.desktop': () => ({ path: `${mfs.HOME}/Desktop` }),
    'fs.list': (p) => mfs.list(String(p.path ?? mfs.HOME), !!p.hidden),
    'fs.trash': (p) => mfs.remove((p.paths as string[]) ?? []),
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
