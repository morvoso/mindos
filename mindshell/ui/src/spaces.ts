// Spaces: desktops the user makes for what they are doing -- Gaming, Work,
// Hobby. Each has its own shortcuts and notes, and may ask for a performance
// mode, a colour palette and a window layout when it is entered. The
// compositor keeps each window on the space it opened on (it calls them
// desks), so switching changes which windows are on screen too; an app on the
// sticky list stays open on all of them.

import * as bridge from './bridge';
import { deepClone, h, newId } from './dom';
import { icon } from './icons';
import { normalizeWorkspace } from './layout';
import { setPerfMode } from './perf';
import { store } from './state';
import type { Layout, Space } from './types';

type Workspace = NonNullable<Layout['desktop']['workspace']>;

/** The icons a space can wear, in the order the picker shows them. */
export const SPACE_ICONS = [
  'gamepad', 'grid', 'briefcase', 'code', 'terminal', 'globe', 'music', 'headphones', 'film', 'image', 'camera',
  'brush', 'edit', 'book', 'office', 'mail', 'coffee', 'heart', 'star', 'trophy', 'rocket', 'flask', 'leaf',
  'sparkle', 'brain', 'home', 'user', 'folder', 'wrench', 'docker', 'cpu', 'moon',
];

export function workspaceOf(layout: Layout = store.state.layout): Workspace {
  return layout.desktop.workspace ?? normalizeWorkspace(undefined);
}

export function spaces(layout?: Layout): Space[] {
  return workspaceOf(layout).spaces;
}

export function activeSpace(layout?: Layout): Space {
  const w = workspaceOf(layout);
  return w.spaces.find((s) => s.id === w.space) ?? w.spaces[0];
}

/** Change the saved workspace (spaces, sticky apps, the active space). */
export function updateWorkspace(mutate: (w: Workspace) => void): Promise<void> {
  return store.updateLayout((l) => {
    const w = l.desktop.workspace ? l.desktop.workspace : (l.desktop.workspace = normalizeWorkspace(undefined));
    mutate(w);
  });
}

/** Change one space in place; nothing happens if it has gone. */
export function updateSpace(id: string, mutate: (s: Space) => void): Promise<void> {
  return updateWorkspace((w) => {
    const s = w.spaces.find((x) => x.id === id);
    if (s) mutate(s);
  });
}

export function switchSpace(id: string): void {
  if (id === workspaceOf().space || !spaces().some((s) => s.id === id)) return;
  void updateWorkspace((w) => { w.space = id; });
}

export function addSpace(name: string, icon?: string): Space {
  const space: Space = { id: newId('space'), name: name.trim().slice(0, 40) || 'New space', ...(icon ? { icon } : {}), notes: '', shortcuts: [] };
  void updateWorkspace((w) => { w.spaces.push(deepClone(space)); });
  return space;
}

/** Remove a space. Its windows move to the space taking its place on screen,
 *  rather than being left on a desk nothing can switch to. */
export function removeSpace(id: string): void {
  const list = spaces();
  if (list.length < 2) return;
  const index = list.findIndex((s) => s.id === id);
  if (index < 0) return;
  const heir = workspaceOf().space === id ? list[index ? index - 1 : 1].id : workspaceOf().space;
  for (const win of store.state.allWindows) {
    if (win.desk === id) bridge.send('wm.moveToDesk', { id: win.id, desk: heir });
  }
  void updateWorkspace((w) => {
    w.spaces = w.spaces.filter((s) => s.id !== id);
    w.space = heir;
  });
}

export function moveSpace(id: string, by: number): void {
  void updateWorkspace((w) => {
    const i = w.spaces.findIndex((s) => s.id === id);
    const j = i + by;
    if (i < 0 || j < 0 || j >= w.spaces.length) return;
    [w.spaces[i], w.spaces[j]] = [w.spaces[j], w.spaces[i]];
  });
}

const same = (a: string, b: string) => a.toLowerCase() === b.toLowerCase();

export function isSticky(appId: string): boolean {
  return !!appId && (workspaceOf().sticky ?? []).some((x) => same(x, appId));
}

/** Keep an app's windows open on every space, or bind them back to one. */
export function setSticky(appId: string, on: boolean): Promise<void> {
  return updateWorkspace((w) => {
    const rest = (w.sticky ?? []).filter((x) => !same(x, appId));
    w.sticky = on && appId ? [...rest, appId] : rest;
  });
}

/** The space's icon, or its initial when it has none (shown where labels are hidden). */
export function spaceGlyph(space: Space, size = 15): HTMLElement | SVGSVGElement {
  return space.icon ? icon(space.icon, size) : h('span', { class: 'space-initial', 'aria-hidden': 'true' }, (space.name.trim()[0] ?? '?').toUpperCase());
}

/**
 * Keep the machine on the active space: the compositor's desk and sticky
 * list always, and the space's performance and window layout when the user
 * moves to it (or changes them on the space they are on). Starting the shell
 * only tells the compositor where it is -- a mode the user picked by hand
 * since the last switch is theirs to keep. Run by the primary desktop only.
 */
export function followSpaces(): () => void {
  let desk = '';
  let sticky = '';
  let perf: string | undefined;
  let layoutMode: string | undefined;
  // The compositor announces every desk change, including the ones asked for
  // here. While a request is on its way those echoes may be stale (A, B, A
  // clicked quickly answers A, B, A), so they are ignored until it lands. The
  // reply says where the compositor ended up, which is the last word.
  let inFlight = 0;
  const setDesk = (params: Record<string, unknown>) => {
    inFlight++;
    void bridge.call<{ desk?: string }>('wm.setDesk', params)
      .then((r) => { if (inFlight === 1 && r?.desk) store.state.desk = r.desk; })
      .catch((e) => console.warn('wm.setDesk', e))
      .finally(() => { inFlight--; });
  };
  const sync = () => {
    const space = activeSpace();
    const list = workspaceOf().sticky ?? [];
    const stickyKey = JSON.stringify(list);
    const first = desk === '';
    const switched = !first && space.id !== desk;
    const params: Record<string, unknown> = {};
    if (switched || stickyKey !== sticky) Object.assign(params, { desk: space.id, sticky: list });
    if (!first && space.layoutMode && (switched || space.layoutMode !== layoutMode)) params.mode = space.layoutMode;
    if (first) Object.assign(params, { desk: space.id, sticky: list });
    if (Object.keys(params).length) setDesk({ desk: space.id, ...params });
    // A game already running holds its own mode until it ends (GameMode's
    // doing), so this is what the machine goes back to, not a fight with it.
    if (!first && space.perf && (switched || space.perf !== perf)) {
      void setPerfMode(space.perf).catch((e) => console.warn('performance mode', e));
    }
    desk = space.id; sticky = stickyKey; perf = space.perf; layoutMode = space.layoutMode;
  };
  // The compositor moved to another space by itself: something asked for a
  // window that lives there (Alt+Tab, a launcher raising a running app). The
  // desktop follows, and the space's own settings come with it.
  const followDesk = () => {
    const at = store.state.desk;
    if (inFlight || !at || at === activeSpace().id) return;
    if (spaces().some((s) => s.id === at)) void updateWorkspace((w) => { w.space = at; });
    // A desk no space has (one deleted in another session): back to ours.
    else setDesk({ desk: activeSpace().id, sticky: workspaceOf().sticky ?? [] });
  };
  sync();
  const offs = [store.on('layout', sync), store.on('desk', followDesk)];
  return () => offs.forEach((off) => off());
}
