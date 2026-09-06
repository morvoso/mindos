// Shared types for the mindshell UI. They mirror docs/SHELL.md.

export type Edge = 'top' | 'bottom' | 'left' | 'right';
export type Align = 'start' | 'center' | 'end';
export type PanelLayer = 'top' | 'bottom';
export type Container = 'panel' | 'desktop';
export type WindowKind = 'desktop' | 'panel' | 'popup' | 'preview' | 'app';

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

export interface Layout {
  version: 1;
  panels: PanelDef[];
  desktop: {
    wallpaper: Wallpaper;
    widgets: DesktopWidgetEntry[];
  };
}

export interface OutputInfo {
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

export interface BatteryState {
  present: boolean;
  percent?: number;
  charging?: boolean;
  timeToEmpty?: number;
}

export interface MindStatus {
  connected: boolean;
  ready: boolean;
  model?: string;
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
  windows: WindowInfo[];
  focused: number | null;
  apps: AppInfo[];
  tray: TrayItem[];
  layout: Layout;
  editMode: boolean;
  config: ShellConfig;
  mind?: MindStatus;
  audio?: AudioState;
  app?: AppMode | null;
  version?: string;
}

/* ----- compositor: layout modes, preferences, outputs ----- */

export interface LayoutModeInfo {
  mode: string;
  label: string;
  modes?: { name: string; label: string }[];
}

export interface Prefs {
  layout_mode?: string | null;
  mind_show_tools?: boolean | null;
  primary_output?: string | null;
  outputs?: Record<string, unknown>;
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
  /** A ready-made thumbnail URL (the mock); the host serves mindos://shell/thumb/ instead. */
  thumb?: string;
}

export interface FsListing {
  path: string;
  parent: string | null;
  entries: FsEntry[];
}

export interface FsStat {
  path: string;
  name: string;
  dir: boolean;
  size: number;
  mtime: number;
  mime: string;
  items?: number;
  link?: string;
  permissions: string;
  mode: number;
}

export interface Place {
  name: string;
  path: string;
  icon: string;
  kind: 'home' | 'folder' | 'system' | 'mount';
  removable?: boolean;
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
  | { pin: { panel: string; widget: string; app: string; pinned: boolean } };

export interface MenuAction {
  label: string;
  icon?: string;
  disabled?: boolean;
  separator?: boolean;
  danger?: boolean;
  action?: Action;
}
