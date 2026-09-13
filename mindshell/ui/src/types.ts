// Shared types for the mindshell UI. They mirror docs/SHELL.md.

export type Edge = 'top' | 'bottom' | 'left' | 'right';
export type Align = 'start' | 'center' | 'end';
export type PanelLayer = 'top' | 'bottom';
export type Container = 'panel' | 'desktop';
export type WindowKind = 'desktop' | 'panel' | 'popup' | 'preview' | 'app' | 'greeter' | 'toast' | 'lock';

export type Config = Record<string, unknown>;

export interface WidgetEntry {
  id: string;
  type: string;
  config: Config;
}

export interface PanelDef {
  id: string;
  output: string;
  edge: Edge;
  size: number;
  length: number;
  align: Align;
  margin: number;
  layer: PanelLayer;
  opacity: number;
  /** Island (inset, rounded) or flush with the edge; unset = by thickness. */
  float?: boolean;
  widgets: WidgetEntry[];
}

export interface DesktopWidgetEntry extends WidgetEntry {
  output: string;
  x: number;
  y: number;
  w: number;
  h: number;
}

export interface Wallpaper {
  mode: 'builtin' | 'image';
  path?: string;
}

export interface AppearancePalette {
  accent: string;
  background: string;
  surface: string;
  surfaceStrong: string;
  border: string;
  text: string;
  muted: string;
}

export interface SavedPalette {
  name: string;
  dark: AppearancePalette;
  light: AppearancePalette;
}

export interface Layout {
  version: number;
  panels: PanelDef[];
  desktop: {
    wallpaper: Wallpaper;
    widgets: DesktopWidgetEntry[];
    /** Show the Desktop folder as icons (default true). */
    icons?: boolean;
    workspace?: {
      /** The id of the space the desktop is on. */
      space: string;
      spaces: Space[];
      /** How a desktop shortcut runs: one click or two. The menu is always one. */
      activate?: 'single' | 'double';
      /** App ids whose windows stay open on every space (Discord, a music player). */
      sticky?: string[];
    };
    appearance?: { theme: 'dark' | 'light'; live?: boolean; dark?: AppearancePalette; light?: AppearancePalette; preset?: string; saved?: SavedPalette[] };
    library?: { favorites: string[]; launched: Record<string, number> };
  };
}

/** A space: a desktop of its own, with its own windows, shortcuts, notes,
 *  performance mode, colours and window layout. Anything unset leaves the
 *  machine as it is when the space is entered. */
export interface Space {
  id: string;
  name: string;
  icon?: string;
  perf?: PerfMode;
  /** `preset:<id>` or `saved:<name>`; unset uses the colours in Appearance. */
  palette?: string;
  /** A compositor layout mode: floating, dwindle or columns. */
  layoutMode?: string;
  /** Show the Resume playing card. */
  recent?: boolean;
  notes: string;
  shortcuts?: WorkspaceShortcut[];
}

export interface WorkspaceShortcut {
  id: string;
  appId: string;
  label: string;
  icon?: string;
  /** Pinned shortcuts also appear in the left menu. */
  pinned?: boolean;
}

export interface OutputInfo {
  primary?: boolean;
  software_rendering?: boolean;
  name: string;
  make?: string;
  model?: string;
  x: number;
  y: number;
  width: number;
  height: number;
  scale: number;
  refresh?: number;
}

export interface WindowInfo {
  id: number;
  title: string;
  app_id: string;
  focused: boolean;
  fullscreen: boolean;
  maximized: boolean;
  minimized: boolean;
  x11: boolean;
  /** A Windows program running under Wine or Proton. */
  wine?: boolean;
  output: string | null;
  /** The space the window belongs to (the compositor calls it a desk). */
  desk?: string;
  /** Hidden because its space is not the one on screen. */
  away?: boolean;
  /** Shown on every space. */
  sticky?: boolean;
}

export interface AppInfo {
  id: string;
  name: string;
  comment?: string;
  exec: string;
  icon: string;
  categories: string[];
  terminal: boolean;
  /** StartupWMClass from the desktop entry, when it names the windows' app_id. */
  wmClass?: string;
  /** The entry starts a Windows program through Wine. */
  wine?: boolean;
}

export interface TrayItem {
  id: string;
  title: string;
  tooltip?: string;
  icon: string;
  status?: string;
  hasMenu: boolean;
  /** A legacy X11 (XEmbed) icon hosted by the compositor: clicks are replayed on it, there is no menu protocol. */
  xembed?: boolean;
}

export interface MenuItem {
  id: number | string;
  label: string;
  enabled: boolean;
  type: 'item' | 'separator' | 'submenu';
  toggle?: 'checkmark' | 'radio';
  checked?: boolean;
  icon?: string;
  children?: MenuItem[];
}

export interface Stats {
  cpu: number;
  memUsed: number;
  memTotal: number;
  gpu?: { util: number; temp?: number; mem?: number; memTotal?: number; name?: string };
  load?: number[];
  uptime?: number;
}

/* The Task Manager's view of the machine, from `system.overview`. Rates are
   per second; sizes are bytes; temperatures are degrees Celsius; clocks are
   MHz. Anything the machine does not measure is absent rather than zero. */

export interface CpuInfo {
  model: string;
  vendor: string;
  cores: number;
  threads: number;
  usage: number;
  perCore: number[];
  kinds: { user: number; system: number; iowait: number; irq: number };
  freq: number[];
  freqAvg?: number | null;
  freqMax?: number | null;
  governor?: string | null;
  driver?: string | null;
  epp?: string | null;
  temp?: number | null;
  tempLabel?: string | null;
  load: number[];
  procs: number;
  running: number;
  ctxtRate: number;
  intrRate: number;
  forkRate: number;
}

export interface MemoryInfo {
  total: number;
  used: number;
  available: number;
  free: number;
  buffers: number;
  cached: number;
  shared: number;
  dirty: number;
  slab: number;
  kernel: number;
  swapTotal: number;
  swapUsed: number;
  swapFree: number;
  zram: { name: string; size: number; stored: number; compressed: number; used: number; algorithm?: string | null }[];
  swaps: { name: string; kind: string; size: number; used: number; priority?: number | null }[];
}

export interface GpuInfo {
  name: string;
  vendor: string;
  util?: number | null;
  memUtil?: number | null;
  mem?: number | null;
  memTotal?: number | null;
  temp?: number | null;
  power?: number | null;
  powerLimit?: number | null;
  clock?: number | null;
  memClock?: number | null;
  fan?: number | null;
  fanPercent?: number | null;
  driver?: string | null;
}

export interface DiskInfo {
  device: string;
  model?: string | null;
  size: number;
  rotational: boolean;
  removable: boolean;
  scheduler?: string | null;
  readRate: number;
  writeRate: number;
  util: number;
  iops: number;
}

export interface FilesystemInfo {
  device: string;
  mount: string;
  fstype: string;
  readOnly: boolean;
  size: number;
  used: number;
  avail: number;
  percent: number;
}

export interface NetInfo {
  iface: string;
  kind: 'ethernet' | 'wifi' | 'vpn' | 'bridge' | 'virtual';
  state: string;
  mac?: string | null;
  mtu?: number | null;
  speed?: number | null;
  addrs: string[];
  rx: number;
  tx: number;
  rxRate: number;
  txRate: number;
  errors: number;
}

export interface SensorInfo {
  temps: { chip: string; label: string; value: number }[];
  fans: { chip: string; label: string; rpm: number }[];
  power: { chip: string; label: string; watts: number }[];
}

export interface HostInfo {
  hostname: string;
  os: string;
  osId: string;
  kernel?: string | null;
  arch: string;
  uptime: number;
  boot: number;
  product?: string | null;
  board?: string | null;
  bios?: string | null;
  packages?: number | null;
  session?: string | null;
  shell: string;
  user: string;
}

export interface ProcessRow {
  pid: number;
  ppid: number;
  name: string;
  state: string;
  cpu: number;
  rss: number;
  vsize: number;
  threads: number;
  prio: number;
  nice: number;
  started: number;
  /* Present only in the full table, not the readout's short list. */
  uid?: number;
  user?: string;
  cmd?: string;
  wine?: boolean;
  own?: boolean;
  readRate?: number;
  writeRate?: number;
}

export interface ProcessTable {
  processes: ProcessRow[];
  total: number;
  matched: number;
  threads: number;
  states: Record<string, number>;
  cores: number;
  uid: number;
}

export interface ProcessDetail {
  pid: number;
  name: string;
  cmd: string;
  exe?: string | null;
  cwd?: string | null;
  state: string;
  ppid?: number | null;
  threads?: number | null;
  vmPeak?: number | null;
  vmSize?: number | null;
  vmRss?: number | null;
  vmSwap?: number | null;
  fds?: number | null;
  read: number;
  written: number;
  cgroup?: string | null;
  wine: boolean;
  voluntary?: number | null;
  involuntary?: number | null;
}

export interface ContainerRow {
  engine: 'docker' | 'podman';
  id: string;
  name: string;
  image: string;
  command: string;
  status: string;
  state: string;
  running: boolean;
  ports: string | number;
  created: string;
  size?: string;
  cpu?: number | null;
  mem?: string | null;
  memPercent?: number | null;
  net?: string | null;
  block?: string | null;
  pids?: string | null;
}

export interface ContainerEngine {
  available: boolean;
  running: boolean;
  containers: ContainerRow[];
  error?: string;
}

export interface Containers {
  docker: ContainerEngine;
  podman: ContainerEngine;
}

export interface UnitRow {
  unit: string;
  load: string;
  active: string;
  sub: string;
  description: string;
}

export interface UnitScope {
  available: boolean;
  running: number;
  failed: UnitRow[];
  units: UnitRow[];
}

export interface Services {
  available: boolean;
  system: UnitScope;
  user: UnitScope;
}

export interface Overview {
  at: number;
  cpu: CpuInfo;
  memory: MemoryInfo;
  gpus: GpuInfo[];
  disks?: DiskInfo[];
  filesystems?: FilesystemInfo[];
  net?: NetInfo[];
  sensors?: SensorInfo;
  host?: HostInfo;
  containers?: Record<'docker' | 'podman', { available: boolean; running: number; total: number }>;
  top?: ProcessRow[];
}

export interface AudioState {
  volume: number;
  muted: boolean;
  sink?: string;
}

export interface NetworkState {
  connected: boolean;
  kind: 'ethernet' | 'wifi' | 'none';
  ssid?: string;
  iface?: string;
  ip?: string;
}

/** A WireGuard tunnel (a NetworkManager connection of type wireguard). */
export interface VpnTunnel {
  /** The connection UUID: what every `vpn.*` call takes. */
  id: string;
  name: string;
  iface?: string;
  /** The tunnel's own address, e.g. 10.66.0.2/24. */
  address?: string;
  /** The first peer's endpoint, host:port. */
  endpoint?: string;
  peers: number;
  active: boolean;
  activating: boolean;
  autoconnect: boolean;
}

export interface VpnState {
  /** False when NetworkManager (nmcli) is not there. */
  available: boolean;
  tunnels: VpnTunnel[];
}

export interface BatteryState {
  present: boolean;
  percent?: number;
  charging?: boolean;
  timeToEmpty?: number;
}

/** What the Mind is allowed to do on its own (Settings › Mind). */
export interface MindPermissions {
  system_changes: boolean;
  aur: boolean;
}

export interface MindStatus {
  connected: boolean;
  ready: boolean;
  model?: string;
  /** The shell's own subscription to mindd is up. */
  daemon?: boolean;
  /** The model is unloaded (a game is running, or `mind sleep on`). */
  sleeping?: boolean;
  notices?: MindNotice[];
  updates?: UpdateStatus | null;
  health?: HealthReport | null;
}

/* ----- the Mind daemon: notices, updates, health ----- */

/** A button on a notice: `chat` opens the Mind bar with `arg`, `request`
 * sends the daemon request in `arg`, `command` runs `arg`, `settings` opens
 * the Settings page named by `arg`. */
export interface NoticeAction {
  label: string;
  kind: 'chat' | 'request' | 'command' | 'settings';
  arg: unknown;
}

export type NoticeLevel = 'info' | 'warn' | 'danger' | 'ok';

export interface MindNotice {
  id: string;
  level: NoticeLevel;
  title: string;
  body: string;
  source: 'updates' | 'health' | 'mind' | string;
  time: number;
  actions: NoticeAction[];
}

export interface PackageUpdate {
  name: string;
  from: string;
  to: string;
  tag: 'kernel' | 'gpu' | 'graphics' | 'core' | 'mindos' | 'gaming' | '';
}

export interface NewsItem {
  title: string;
  date: string;
  url: string;
}

export interface LastUpdate {
  time: number;
  packages: string[];
  pre_snapshot: number | null;
  ok: boolean;
  verified: '' | 'ok' | 'problems';
  report: string;
}

export interface UpdateStatus {
  checked_at: number;
  packages: PackageUpdate[];
  news: NewsItem[];
  risk: 'low' | 'medium' | 'high' | '';
  summary: string;
  warnings: string[];
  manual_intervention: boolean;
  reboot: boolean;
  assessed_by_model: boolean;
  assessing: boolean;
  checking: boolean;
  applying: boolean;
  auto_apply: boolean;
  last_update: LastUpdate | null;
  error: string;
}

export interface Finding {
  id: string;
  level: NoticeLevel;
  title: string;
  body: string;
  actions: NoticeAction[];
}

export interface HealthReport {
  checked_at: number;
  findings: Finding[];
}

export interface Snapshot {
  number: number;
  type: string;
  date: string;
  description: string;
  cleanup?: string;
}

/* ----- notifications (org.freedesktop.Notifications) ----- */

export interface NotificationAction {
  key: string;
  label: string;
}

export interface Notification {
  id: number;
  app: string;
  /** Desktop entry id without `.desktop`, when the app said. */
  desktop: string;
  /** Resolved icon URL, or empty. */
  icon: string;
  summary: string;
  body: string;
  actions: NotificationAction[];
  /** 0 low, 1 normal, 2 critical. */
  urgency: number;
  resident: boolean;
  transient: boolean;
  category: string;
  /** ms; -1 = server default, 0 = never. */
  timeout: number;
  time: number;
  replaced: boolean;
  /** Arrived while Do not disturb was on: kept, not shown as a toast. */
  quiet?: boolean;
}

export interface NotifyState {
  items: Notification[];
  dnd: boolean;
}

/* ----- polkit: the authentication dialog ----- */

/** An authorisation the polkit agent is waiting for (`shell.state.polkit`). */
export interface PolkitRequest {
  id: number;
  /** The polkit action, e.g. org.freedesktop.systemd1.manage-units. */
  action: string;
  message: string;
  icon: string;
  /** The account that will authenticate. */
  user: string;
  /** Every account polkit would accept. */
  users: string[];
  /** The command behind a pkexec call, when polkit knows it. */
  command: string;
  /** Why the last attempt failed, empty on the first one. */
  error: string;
  attempt: number;
  tries: number;
  /** The password went to PAM and the answer is not in yet. */
  busy: boolean;
}

/* ----- performance modes (mindos-perf) ----- */

export type PerfMode = 'balanced' | 'performance' | 'quiet';

export interface PerfStatus {
  mode: PerfMode;
  effective: PerfMode | '';
  game: number;
  gameMode: PerfMode | '';
  mindSleeps: boolean;
  cpu: string;
  driver: string;
  governor: string;
  epp: string;
  boost: boolean | null;
  platformProfile: string;
  thp: string;
  scheduler: string;
  scx: string;
  nvidia: boolean;
  gpu: string;
  powerLimit: string;
  powerLimitPolicy: string;
  persistence: boolean;
}

/* ----- shell.run ----- */

export interface RunResult {
  status: number;
  ok: boolean;
  stdout: string;
  stderr: string;
  json: unknown;
}

/* ----- DLSS swapper (mindos-dlss) ----- */

export interface DlssDll {
  kind: string;
  label: string;
  file: string;
  version: string;
  swapped: boolean;
  changed?: boolean;
  restorable?: boolean;
  backup_version?: string;
}

export interface DlssGame {
  id: string;
  name: string;
  source: 'steam' | 'heroic' | 'lutris' | 'dir' | string;
  path: string;
  dlls: DlssDll[];
}

export interface DlssLibraryEntry {
  kind: string;
  label: string;
  version: string;
  path: string;
  source: string;
  size: number;
}

export interface DlssVersion {
  version: string;
  installed: boolean;
  label: string;
  dev: boolean;
  signed: string;
  size: number;
  description: string;
}

export interface DlssKind {
  kind: string;
  dll: string;
  label: string;
}

export interface ShellConfig {
  icon_theme?: string;
  hardware_acceleration?: string;
  terminal?: string;
  icon_size?: number;
}

/** Which app an `app` window shows (`mindshell --app NAME --page PAGE ARG`). */
export interface AppMode {
  name: string;
  page?: string;
  arg?: string;
}

export interface ShellState {
  user: string;
  host: string;
  uptime?: number;
  outputs: OutputInfo[];
  /** The windows on screen: every space's but the ones away on another space. */
  windows: WindowInfo[];
  /** Every window, including those on other spaces. */
  allWindows: WindowInfo[];
  /** The space the compositor is showing, as it last reported it. */
  desk: string;
  focused: number | null;
  apps: AppInfo[];
  tray: TrayItem[];
  layout: Layout;
  editMode: boolean;
  /** The primary screen is showing the home screen rather than the windows. */
  desktopHome: boolean;
  config: ShellConfig;
  mind?: MindStatus;
  notify?: NotifyState;
  polkit?: PolkitRequest | null;
  audio?: AudioState;
  /** Read on demand (`vpn.list`) and pushed as `vpn`. */
  vpn?: VpnState;
  /** Pushed as `network` whenever NetworkManager reports a change. */
  network?: NetworkState;
  app?: AppMode | null;
  version?: string;
  /** A game is running: the shell keeps still until it ends. */
  game?: boolean;
  /** The screensaver / lock stage, as the compositor reports it. */
  lock?: LockState;
}

/* ----- compositor: layout modes, preferences, outputs ----- */

export interface LayoutModeInfo {
  mode: string;
  label: string;
  modes?: { name: string; label: string }[];
}

export interface Prefs {
  input?: Partial<InputSettings>;
  layout_mode?: string | null;
  mind_show_tools?: boolean | null;
  primary_output?: string | null;
  idle?: Partial<IdlePrefs>;
  outputs?: Record<string, unknown>;
}

export interface InputSettings {
  keyboard_layout: string;
  keyboard_variant: string;
  keyboard_options: string;
  repeat_rate: number;
  repeat_delay: number;
  mouse_profile: string;
  mouse_speed: number;
  mouse_left_handed: boolean;
  mouse_natural_scroll: boolean;
}
export interface InputState {
  settings: InputSettings;
  mice: { name: string; acceleration: boolean; profiles: string[]; profile: string | null;
    speed: number; left_handed: boolean; natural_scroll: boolean }[];
}

/** What happens when the machine is left alone (Settings › Screen). Every
 *  timeout is in seconds, counted from the last key, click or gesture; `0`
 *  means never. */
export interface IdlePrefs {
  screensaver: number;
  /** A saver id, `shuffle` or `blank`. */
  saver: string;
  lock: number;
  blank: number;
  lock_on_blank: boolean;
  lock_on_sleep: boolean;
  stay_awake_when_busy: boolean;
}

/** Where the session stands, from the compositor's `idle` event. */
export interface LockState {
  /** `active`, `screensaver` or `blank`. */
  stage: string;
  locked: boolean;
  /** Something is holding the session awake (a video, a game). */
  inhibited: boolean;
  saver: string;
}

export interface WmMode {
  width: number;
  height: number;
  /** Millihertz. */
  refresh: number;
  preferred: boolean;
  current: boolean;
}

/** An output as the compositor reports it (`get_outputs`). */
export interface WmOutput {
  name: string;
  make: string;
  model: string;
  x: number;
  y: number;
  width: number;
  height: number;
  scale: number;
  /** Hertz. */
  refresh: number;
  transform: string;
  modes: WmMode[];
  enabled: boolean;
  vrr: boolean;
  vrr_supported: boolean;
  primary: boolean;
  mm_width: number;
  mm_height: number;
}

/** `pointer.get` / `pointer.set`: the cursor theme and size. */
export interface PointerState {
  theme: string;
  size: number;
  themes: string[];
  sizes: number[];
  /** False when gsettings-desktop-schemas is missing: nothing can be saved. */
  writable: boolean;
}

/** What `wm.setOutput` accepts besides `name`. */
export interface OutputChange {
  mode?: { width: number; height: number; refresh: number };
  scale?: number;
  position?: [number, number];
  transform?: string;
  enabled?: boolean;
  vrr?: boolean;
  primary?: boolean;
}

/* ----- Mind (mindd) ----- */

export interface ModelEntry {
  file: string;
  path: string;
  size: number;
  active: boolean;
}

export interface CatalogEntry {
  id: string;
  name: string;
  file: string;
  url: string;
  size: number;
  license: string;
  license_url: string;
  params: string;
  description: string;
  min_vram_gb: number;
  recommended: boolean;
  installed: boolean;
}

export interface DownloadState {
  file: string;
  url: string;
  received: number;
  total: number;
  done: boolean;
  error?: string | null;
}

export interface ModelsInfo {
  type?: string;
  current: string | null;
  model: string;
  ready: boolean;
  auto: boolean;
  thinking: boolean;
  models_dir: string;
  external: boolean;
  gpu_memory: number | null;
  models: ModelEntry[];
  catalog: CatalogEntry[];
  download: DownloadState | null;
}

/* ----- files ----- */

export interface FsEntry {
  name: string;
  path: string;
  dir: boolean;
  size: number;
  mtime: number;
  hidden: boolean;
  symlink: boolean;
  mime: string;
  icon: string;
  image: boolean;
  /** A `.desktop` or Windows `.lnk` shortcut: `label` is what it points at. */
  shortcut?: boolean;
  label?: string | null;
  /** A ready-made thumbnail URL (the mock); the host serves mindos://shell/thumb/ instead. */
  thumb?: string;
}

export interface FsListing {
  path: string;
  parent: string | null;
  entries: FsEntry[];
}

export interface WallpaperEntry {
  path: string;
  name: string;
  folder: string;
  thumb?: string;
}

/** A rectangle in output (logical) coordinates plus the edge it hangs off. */
export interface Anchor {
  x: number;
  y: number;
  w: number;
  h: number;
  edge?: Edge;
}

/** Something a menu entry or button can do; runs through actions.ts. */
export type Action =
  | { call: string; params?: Record<string, unknown> }
  | { popup: string; arg?: unknown; keyboard?: boolean }
  | { editMode: boolean }
  | { exec: string }
  | { pin: { panel: string; widget: string; app: string; pinned: boolean } }
  | { desktopIcons: boolean }
  | { removeWidget: { kind: Container; panel?: string; widget: string } }
  | { shortcut: { id: string; op: 'pin' | 'remove' } }
  | { sticky: { app: string; on: boolean } };

export interface MenuAction {
  label: string;
  icon?: string;
  disabled?: boolean;
  separator?: boolean;
  danger?: boolean;
  action?: Action;
}
