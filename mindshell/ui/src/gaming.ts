/** Real host-backed gaming services. Browser preview deliberately has no account data. */
import * as bridge from './bridge';
export interface Session { id: string; game: string; started: number; ended?: number; active: boolean; suspended: boolean; transition_ms?: number; stats?: { avg_fps: number; p99_ms: number; low_fps: number; stutters: number; gpu_temp?: number; points: number[] } }
export interface GameMeta { save_path?: string; completion?: number; chapter?: string; notes?: string; wiki?: string; video?: string; fps_limit?: number }
export function play<T = unknown>(action: string, params: Record<string, unknown> = {}): Promise<T> { return bridge.call<T>('gaming.request', { ...params, action }); }
export function openGaming(page = 'sessions', game = ''): Promise<unknown> { return bridge.call('shell.openApp', { name: 'gaming', page, arg: game }); }
export function openCompanion(game = ''): Promise<unknown> { return bridge.call('shell.openApp', { name: 'companion', arg: game }); }
