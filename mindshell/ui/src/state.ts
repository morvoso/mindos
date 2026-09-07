// The shell state store: one per page, filled from `shell.state` and kept
// current by host events. Components subscribe to the keys they render.

import * as bridge from './bridge';
import { deepClone } from './dom';
import { normalizeLayout } from './layout';
import type { AudioState, HealthReport, Layout, LayoutModeInfo, MindNotice, MindStatus, Notification, NotifyState, OutputInfo, PolkitRequest, Prefs, ShellState, TrayItem, UpdateStatus, WindowInfo, AppInfo } from './types';

export type StateKey = 'windows' | 'outputs' | 'apps' | 'tray' | 'layout' | 'editMode' | 'mind' | 'audio' | 'popups' | 'shortcut' | 'layoutMode' | 'prefs' | 'notify' | 'mindNotices' | 'mindUpdates' | 'mindHealth' | 'polkit';

type Cb = (state: ShellState) => void;

export class Store {
  state!: ShellState;
  /** Names of popups currently open (per the host's popup_state events). */
  popups = new Set<string>();
  lastShortcut = '';
  /** The compositor's window layout mode, once it has been reported. */
  layoutMode?: LayoutModeInfo;
  /** Compositor preferences (layout mode, Mind tool lines, primary output). */
  prefs: Prefs = {};
  /** Set for the duration of a `notify` / `mindNotices` emit: what just arrived. */
  notifyAdded?: Notification;
  notifyClosed?: number;
  noticeAdded?: MindNotice;
  private listeners = new Map<StateKey, Set<Cb>>();
  private ready = false;

  async init(): Promise<ShellState> {
    if (this.ready) return this.state;
    let raw: Partial<ShellState> = {};
    try {
      raw = await bridge.call<ShellState>('shell.state');
    } catch (e) {
      console.warn('shell.state failed, using empty state', e);
    }
    this.state = {
      user: raw.user ?? '',
      host: raw.host ?? '',
      uptime: raw.uptime,
      outputs: raw.outputs ?? [],
      windows: raw.windows ?? [],
      focused: raw.focused ?? null,
      apps: raw.apps ?? [],
      tray: raw.tray ?? [],
      layout: normalizeLayout(raw.layout),
      editMode: !!raw.editMode,
      config: raw.config ?? {},
      mind: raw.mind,
      notify: raw.notify ?? { items: [], dnd: false },
      audio: raw.audio,
      app: raw.app ?? null,
      polkit: raw.polkit ?? null,
      version: raw.version,
    };
    bridge.on<{ windows: WindowInfo[]; focused: number | null }>('windows', (p) => {
      this.state.windows = p.windows ?? [];
      this.state.focused = p.focused ?? this.state.windows.find((w) => w.focused)?.id ?? null;
      this.emit('windows');
    });
    bridge.on<{ outputs: OutputInfo[] }>('outputs', (p) => {
      this.state.outputs = p.outputs ?? [];
      this.emit('outputs');
    });
    bridge.on<{ apps: AppInfo[] }>('apps', (p) => {
      this.state.apps = p.apps ?? [];
      this.emit('apps');
    });
    bridge.on<{ items: TrayItem[] }>('tray', (p) => {
      this.state.tray = p.items ?? [];
      this.emit('tray');
    });
    bridge.on<{ layout: Layout }>('layout', (p) => {
      this.state.layout = normalizeLayout(p.layout);
      this.emit('layout');
    });
    bridge.on<{ enabled: boolean }>('edit_mode', (p) => {
      this.state.editMode = !!p.enabled;
      this.emit('editMode');
    });
    bridge.on<MindStatus>('mind', (p) => {
      this.state.mind = p;
      this.emit('mind');
    });
    bridge.on<NotifyState & { added?: Notification | null; closed?: number | null }>('notify', (p) => {
      this.state.notify = { items: p.items ?? [], dnd: !!p.dnd };
      this.notifyAdded = p.added ?? undefined;
      this.notifyClosed = p.closed ?? undefined;
      this.emit('notify');
      this.notifyAdded = undefined;
      this.notifyClosed = undefined;
    });
    bridge.on<{ notices: MindNotice[]; added?: MindNotice | null }>('mind_notices', (p) => {
      if (this.state.mind) this.state.mind.notices = p.notices ?? [];
      this.noticeAdded = p.added ?? undefined;
      this.emit('mindNotices');
      this.noticeAdded = undefined;
    });
    bridge.on<UpdateStatus>('mind_updates', (p) => {
      if (this.state.mind) this.state.mind.updates = p;
      this.emit('mindUpdates');
    });
    bridge.on<PolkitRequest | null>('polkit', (p) => {
      this.state.polkit = p && typeof p === 'object' ? p : null;
      this.emit('polkit');
    });
    bridge.on<HealthReport>('mind_health', (p) => {
      if (this.state.mind) this.state.mind.health = p;
      this.emit('mindHealth');
    });
    bridge.on<AudioState>('audio', (p) => {
      this.state.audio = p;
      this.emit('audio');
    });
    bridge.on<{ name: string; open: boolean }>('popup_state', (p) => {
      if (p.open) this.popups.add(p.name);
      else this.popups.delete(p.name);
      this.emit('popups');
    });
    bridge.on<{ name: string }>('shortcut', (p) => {
      this.lastShortcut = p.name;
      this.emit('shortcut');
    });
    bridge.on<LayoutModeInfo>('layout_mode', (p) => {
      this.layoutMode = { mode: p.mode, label: p.label, modes: p.modes ?? this.layoutMode?.modes };
      this.emit('layoutMode');
    });
    bridge.on<{ prefs: Prefs }>('prefs', (p) => {
      this.prefs = p.prefs ?? {};
      this.emit('prefs');
    });
    this.ready = true;
    return this.state;
  }

  on(key: StateKey, cb: Cb): () => void {
    let set = this.listeners.get(key);
    if (!set) this.listeners.set(key, (set = new Set()));
    set.add(cb);
    return () => set!.delete(cb);
  }

  /** Subscribe until `el` leaves the document. */
  bind(el: Element, key: StateKey, cb: Cb): void {
    const off = this.on(key, (s) => {
      if (!el.isConnected) return off();
      cb(s);
    });
  }

  emit(key: StateKey): void {
    this.listeners.get(key)?.forEach((cb) => cb(this.state));
  }

  /** Change the layout: apply locally for instant feedback, then persist through the host. */
  async updateLayout(mutate: (layout: Layout) => void): Promise<void> {
    const next = deepClone(this.state.layout);
    mutate(next);
    this.state.layout = next;
    this.emit('layout');
    try {
      await bridge.call('layout.save', { layout: next });
    } catch (e) {
      console.warn('layout.save failed', e);
    }
  }

  async setEditMode(enabled: boolean): Promise<void> {
    this.state.editMode = enabled;
    this.emit('editMode');
    try {
      await bridge.call('shell.setEditMode', { enabled });
    } catch (e) {
      console.warn('shell.setEditMode failed', e);
    }
  }

  /** Ask the compositor for the layout mode (used before the first event arrives). */
  async fetchLayoutMode(): Promise<LayoutModeInfo | undefined> {
    try {
      const r = await bridge.call<LayoutModeInfo>('wm.layoutMode');
      if (r && r.mode) {
        this.layoutMode = { ...r, modes: r.modes ?? this.layoutMode?.modes };
        this.emit('layoutMode');
      }
    } catch (e) {
      console.warn('wm.layoutMode failed', e);
    }
    return this.layoutMode;
  }

  async fetchPrefs(): Promise<Prefs> {
    try {
      const r = await bridge.call<{ prefs: Prefs }>('prefs.get');
      this.prefs = r?.prefs ?? {};
      this.emit('prefs');
    } catch (e) {
      console.warn('prefs.get failed', e);
    }
    return this.prefs;
  }

  /** Change compositor preferences; the reply (and the `prefs` event) carry the merged result. */
  async setPrefs(patch: Prefs): Promise<Prefs> {
    this.prefs = { ...this.prefs, ...patch };
    this.emit('prefs');
    try {
      const r = await bridge.call<{ prefs: Prefs }>('prefs.set', { prefs: patch });
      if (r?.prefs) {
        this.prefs = r.prefs;
        this.emit('prefs');
      }
    } catch (e) {
      console.warn('prefs.set failed', e);
    }
    return this.prefs;
  }

  output(name: string): OutputInfo | undefined {
    return this.state.outputs.find((o) => o.name === name) ?? this.state.outputs[0];
  }

  app(id: string): AppInfo | undefined {
    return this.state.apps.find((a) => a.id === id);
  }

  notices(): MindNotice[] {
    return this.state.mind?.notices ?? [];
  }

  notifications(): Notification[] {
    return this.state.notify?.items ?? [];
  }

  /** What the bell badge counts: app notifications plus notices that need attention. */
  attention(): number {
    return this.notifications().length + this.notices().filter((n) => n.level === 'warn' || n.level === 'danger').length;
  }
}

export const store = new Store();
