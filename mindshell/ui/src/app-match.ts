// Matching a window back to the application that started it, and launching an
// application with visible feedback while it starts. The task bar, the desktop
// shortcuts and the desktop files all need the same answers.

import * as bridge from './bridge';
import { store } from './state';
import type { AppInfo } from './types';

export const norm = (s: string) => s.toLowerCase().replace(/\.desktop$/, '');
export const last = (s: string) => {
  const parts = s.split('.');
  return parts[parts.length - 1];
};
export const execBase = (exec: string) => {
  const first = exec.trim().split(/\s+/)[0] ?? '';
  return first.split('/').pop() ?? '';
};

/** Index apps by the names a window's app_id is likely to carry. */
export function appIndex(apps: AppInfo[]): Map<string, AppInfo> {
  const idx = new Map<string, AppInfo>();
  const put = (k: string, a: AppInfo) => {
    if (k && !idx.has(k)) idx.set(k, a);
  };
  // StartupWMClass is the entry's own statement of what its windows are
  // called, so it wins over the guesses below (Wine's generated entries rely
  // on it: the id is a menu path, the windows carry the exe name).
  for (const a of apps) if (a.wmClass) put(norm(a.wmClass), a);
  for (const a of apps) if (a.wmClass) put(norm(a.wmClass).replace(/\.exe$/, ''), a);
  for (const a of apps) put(norm(a.id), a);
  for (const a of apps) put(last(norm(a.id)), a);
  for (const a of apps) put(norm(execBase(a.exec)), a);
  for (const a of apps) put(norm(a.name), a);
  return idx;
}

export function matchApp(idx: Map<string, AppInfo>, appId: string): AppInfo | undefined {
  const k = norm(appId);
  return idx.get(k) ?? idx.get(last(k)) ?? idx.get(k.replace(/-bin$|-wayland$|\.exe$/, ''));
}

/** Windows already on screen that belong to this application. */
function windowsFor(app: AppInfo | undefined, appId: string): number {
  const idx = appIndex(store.state.apps);
  const want = app ?? store.state.apps.find((a) => a.id === appId);
  let n = 0;
  for (const w of store.state.windows) {
    const m = matchApp(idx, w.app_id);
    if (want ? m === want : norm(w.app_id) === norm(appId)) n++;
  }
  return n;
}

/** How long the spinner keeps going before we assume the launch failed. */
const LAUNCH_TIMEOUT = 20000;

/**
 * Launch an application and mark `el` as busy until one of its windows shows
 * up. Feedback matters more than accuracy here: a window we fail to match just
 * means the mark clears on the timeout instead of on the window.
 */
export function launchWithFeedback(el: HTMLElement, appId: string): Promise<void> {
  const app = store.state.apps.find((a) => a.id === appId);
  const before = windowsFor(app, appId);
  el.classList.add('launching');
  let done = false;
  const clear = () => {
    if (done) return;
    done = true;
    off();
    clearTimeout(timer);
    el.classList.remove('launching');
  };
  const off = store.on('windows', () => {
    if (windowsFor(app, appId) > before) clear();
  });
  const timer = setTimeout(clear, LAUNCH_TIMEOUT);
  return bridge
    .call('apps.launch', { id: appId })
    .then(() => undefined)
    .catch((e) => {
      clear();
      throw e;
    });
}
