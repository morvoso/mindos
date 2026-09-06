// The bridge to the mindshell host (window.mindos). The host injects a
// bootstrap object before any script runs; if it only provides the WebKit
// message handler, this module installs the same API on top of it. Outside
// the host (a browser) mock.ts installs a fake host instead.

import type { WindowKind } from './types';

export interface WindowInfo {
  kind: WindowKind;
  id: string;
  output: string;
  popup?: string;
  arg?: unknown;
}

type Listener = (payload: unknown) => void;

export interface MindosGlobal {
  call(method: string, params?: Record<string, unknown>): Promise<unknown>;
  on(event: string, cb: Listener): () => void;
  window: WindowInfo;
  _reply?(id: number, ok: boolean, payload: unknown): void;
  _dispatch?(event: string, payload: unknown): void;
}

declare global {
  interface Window {
    mindos?: MindosGlobal;
    webkit?: { messageHandlers?: Record<string, { postMessage(msg: string): void }> };
  }
}

export function parseWindowInfo(): WindowInfo {
  const q = new URLSearchParams(location.search);
  const kind = (q.get('kind') as WindowKind | null) ?? 'preview';
  let arg: unknown;
  const rawArg = q.get('arg');
  if (rawArg) {
    try {
      arg = JSON.parse(rawArg);
    } catch {
      arg = undefined;
    }
  }
  return {
    kind,
    id: q.get('id') ?? '',
    output: q.get('output') ?? '',
    popup: q.get('popup') ?? undefined,
    arg,
  };
}

export function hasHost(): boolean {
  return !!window.webkit?.messageHandlers?.mindos || typeof window.mindos?.call === 'function';
}

/** Return the host bridge, installing the transport over the WebKit message handler when the host did not. */
export function installTransport(): MindosGlobal {
  const existing = window.mindos;
  if (existing && typeof existing.call === 'function' && typeof existing.on === 'function') {
    if (!existing.window || !existing.window.kind) existing.window = parseWindowInfo();
    return existing;
  }
  const handler = window.webkit?.messageHandlers?.mindos;
  const pending = new Map<number, { resolve: (v: unknown) => void; reject: (e: Error) => void }>();
  const listeners = new Map<string, Set<Listener>>();
  let seq = 0;
  const m: MindosGlobal = {
    window: existing?.window ?? parseWindowInfo(),
    call(method, params) {
      if (!handler) return Promise.reject(new Error('mindshell host not available'));
      const id = ++seq;
      return new Promise((resolve, reject) => {
        pending.set(id, { resolve, reject });
        handler.postMessage(JSON.stringify({ id, method, params: params ?? {} }));
      });
    },
    on(event, cb) {
      let set = listeners.get(event);
      if (!set) listeners.set(event, (set = new Set()));
      set.add(cb);
      return () => set!.delete(cb);
    },
    _reply(id, ok, payload) {
      const p = pending.get(id);
      if (!p) return;
      pending.delete(id);
      if (ok) p.resolve(payload);
      else p.reject(new Error(typeof payload === 'string' ? payload : JSON.stringify(payload)));
    },
    _dispatch(event, payload) {
      listeners.get(event)?.forEach((cb) => cb(payload));
      listeners.get('*')?.forEach((cb) => cb({ event, payload }));
    },
  };
  window.mindos = m;
  return m;
}

let api: MindosGlobal | undefined;

function get(): MindosGlobal {
  if (!api) api = installTransport();
  return api;
}

/** Typed request to the host. */
export function call<T = unknown>(method: string, params?: Record<string, unknown>): Promise<T> {
  return get().call(method, params) as Promise<T>;
}

/** Fire-and-forget request; errors are logged, never thrown. */
export function send(method: string, params?: Record<string, unknown>): void {
  get()
    .call(method, params)
    .catch((e: unknown) => console.warn(`mindos.call(${method}) failed:`, e));
}

export function on<T = unknown>(event: string, cb: (payload: T) => void): () => void {
  return get().on(event, cb as Listener);
}

export function windowInfo(): WindowInfo {
  return get().window;
}

export function setApi(m: MindosGlobal): void {
  api = m;
  window.mindos = m;
}
