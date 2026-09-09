import { appearanceControls } from './appearance';
import * as bridge from './bridge';
import { every, gib, h } from './dom';
import { renderGameLibrary } from './game-library';
import { openGaming } from './gaming';
import { icon } from './icons';
import { modeInfo, perfRefresh, perfSubscribe, perfSwitch } from './perf';
import { store } from './state';
import { systemControls } from './system-menu';
import type { PerfStatus, Stats } from './types';

export function renderGamingDesktop(root: HTMLElement, output: string): { el: HTMLElement; destroy: () => void } {
  const workspace = h('div', { class: 'gaming-workspace' });
  const feedback = h('div', { class: 'gaming-feedback', role: 'status', 'aria-live': 'polite', hidden: true });
  const status = h('span', { class: 'gaming-meta gaming-top-status' }, 'MindOS / Desktop session');
  const appearance = appearanceControls();
  const systemMenu = systemControls();
  const header = h('header', { class: 'gaming-menubar' },
    h('strong', { class: 'gaming-brand' }, h('i'), 'MINDOS'),
    h('span', { class: 'gaming-meta gaming-edition' }, '// Library'), h('div', { class: 'header-actions' }, appearance.el, systemMenu.el));
  const nav = h('nav', { class: 'gaming-nav', 'aria-label': 'Desktop shortcuts' });
  const library = h('section', { class: 'gaming-main' });
  const rail = h('aside', { class: 'gaming-rail', 'aria-label': 'Gaming and system tools' });
  workspace.append(header, nav, library, rail, feedback);
  root.append(workspace);
  const report = (message: string) => { feedback.textContent = message; feedback.hidden = !message; };
  let alive = true, statsBusy = false;
  const act = async (fn: () => Promise<unknown>) => { try { await fn(); report(''); } catch (e) { if (alive) report(String(e)); } };
  const settings = (page: string) => bridge.call('shell.openApp', { name: 'settings', page });
  const app = (pattern: RegExp) => {
    const found = store.state.apps.find((a) => pattern.test(a.id));
    return found ? bridge.call('apps.launch', { id: found.id }) : settings('software');
  };
  const toggleLibrary = () => {
    library.hidden = !library.hidden;
    workspace.classList.toggle('library-hidden', library.hidden);
    libraryLink.setAttribute('aria-pressed', String(!library.hidden));
  };
  const link = (name: string, glyph: string, fn: () => void) => h('button', { class: 'gaming-nav-link', onclick: fn }, icon(glyph, 18), h('span', {}, name));
  const libraryLink = link('Library', 'gamepad', toggleLibrary);
  libraryLink.setAttribute('aria-pressed', 'true');
  nav.append(h('span', { class: 'gaming-meta' }, 'Desktop'), libraryLink,
    link('Gaming', 'gamepad', () => void act(() => openGaming())),
    link('Files', 'folder', () => void act(() => bridge.call('fs.open', { path: '~' }))),
    link('Browser', 'globe', () => void act(() => app(/firefox|chromium/))),
    link('Settings', 'gear', () => void act(() => settings('shell'))),
    h('span', { class: 'gaming-nav-bottom gaming-meta' }, 'Super + Space', h('br'), 'Launch · Ask · Find'));

  const section = (name: string, sub: string, body: HTMLElement) => h('section', { class: 'gaming-rail-card' },
    h('header', { class: 'gaming-panel-title' }, h('h2', {}, name), h('span', { class: 'gaming-meta' }, sub)), body);
  const metrics = h('div', { class: 'gaming-metrics' });
  const metric = (name: string, value: string, percent?: number) => h('div', { class: 'gaming-metric' },
    h('div', {}, h('span', { class: 'gaming-meta' }, name), h('span', { class: 'gaming-meta' }, value)),
    h('div', { class: 'gaming-meter' }, h('span', { style: { width: `${Math.max(0, Math.min(100, percent || 0))}%` } })));
  metrics.append(h('p', { class: 'gaming-meta' }, 'Waiting for system readings…'));
  const profile = h('div', { class: 'gaming-profiles' });
  const profileLabel = h('p', { class: 'gaming-meta' });
  let perf: PerfStatus | undefined, switching = false;
  const renderPerf = () => {
    profileLabel.textContent = perf ? `${modeInfo(perf.effective).label}${perf.game ? ' / GameMode active' : ' / System profile'}` : 'Performance controls unavailable';
    profile.replaceChildren(...(['quiet', 'balanced', 'performance'] as const).map((mode) => h('button', {
      class: 'btn', 'aria-pressed': String(perf?.mode === mode), disabled: switching || !perf,
      onclick: async () => {
        switching = true; renderPerf();
        try { const result = await perfSwitch(mode); if (alive) report(result); }
        catch (e) { if (alive) report(String(e)); }
        finally { switching = false; if (alive) renderPerf(); }
      },
    }, mode === 'performance' ? 'Max' : modeInfo(mode).label)));
  };
  const system = h('div', { class: 'gaming-rail-body' }, metrics, profileLabel, profile,
    h('button', { class: 'gaming-text-action', onclick: () => void act(() => settings('performance')) }, 'Performance settings', icon('arrow-right', 12)));
  const launchers = h('div', { class: 'gaming-rail-body gaming-launchers' });
  const renderLaunchers = () => {
    launchers.replaceChildren(...[
      ['Steam', /^(steam|com\.valvesoftware\.Steam)\.desktop$/i, 'Games & downloads'],
      ['Heroic', /heroic|com\.heroicgameslauncher/i, 'Epic & GOG'],
      ['Lutris', /lutris/i, 'Games & runners'],
      ['Discord', /discord/i, 'Voice & friends'],
    ].map(([name, match, desc]) => {
      const found = store.state.apps.find((a) => (match as RegExp).test(a.id));
      return h('button', { class: 'gaming-launcher', onclick: () => void act(() => app(match as RegExp)) },
        h('span', {}, h('strong', {}, String(name)), h('small', {}, String(desc))),
        h('span', { class: 'gaming-meta' }, found ? 'Open ↗' : 'Get ↗'));
    }));
  };
  rail.append(section('System', 'Live readings', system), section('Launchers', 'Connected locally', launchers),
    h('p', { class: 'gaming-rail-foot gaming-meta' }, ''));
  const sample = async () => {
    if (statsBusy || store.state.game || document.hidden || store.state.editMode) return;
    statsBusy = true;
    try {
      const s = await bridge.call<Stats>('system.stats');
      if (!alive || store.state.game) return;
      metrics.replaceChildren(metric('GPU', s.gpu ? `${Math.round(s.gpu.util)}%${s.gpu.temp != null ? ` · ${Math.round(s.gpu.temp)}°` : ''}` : 'Unavailable', s.gpu?.util),
        metric('CPU', `${Math.round(s.cpu)}%`, s.cpu),
        metric('Memory', `${gib(s.memUsed)} / ${gib(s.memTotal)} GB`, s.memTotal ? s.memUsed / s.memTotal * 100 : 0));
      status.textContent = `${s.gpu ? `GPU ${Math.round(s.gpu.util)}%${s.gpu.temp != null ? ` · ${Math.round(s.gpu.temp)}°` : ''}` : 'GPU unavailable'} / CPU ${Math.round(s.cpu)}%`;
    } catch { if (alive) { status.textContent = 'Telemetry unavailable'; metrics.replaceChildren(h('p', { class: 'gaming-meta' }, 'System readings unavailable')); } }
    finally { statsBusy = false; }
  };
  const layout = () => {
    workspace.hidden = store.state.editMode;
    root.classList.toggle('gaming-active', !store.state.editMode);
    const pads = { top: 0, right: 0, bottom: 80, left: 0 };
    for (const p of store.state.layout.panels) if (p.output === '*' || p.output === output) pads[p.edge] = Math.max(pads[p.edge], p.size + p.margin * 2);
    for (const edge of ['top', 'right', 'bottom', 'left'] as const) workspace.style.setProperty(`--gaming-${edge}`, `${pads[edge]}px`);
  };
  const gameState = () => {
    if (store.state.game) { status.textContent = 'GameMode active / Desktop at rest'; metrics.replaceChildren(h('p', { class: 'gaming-meta' }, 'Sampling paused while gaming')); }
    else void sample();
  };
  layout(); renderLaunchers(); renderPerf(); gameState();
  const disposeLibrary = renderGameLibrary(library, toggleLibrary);
  const offs = [store.on('editMode', layout), store.on('layout', layout), store.on('apps', renderLaunchers), store.on('game', gameState),
    perfSubscribe(workspace, (s) => { perf = s; renderPerf(); }), every(workspace, 3000, () => void sample()),
    every(workspace, 15000, () => { if (!store.state.game && !document.hidden) void perfRefresh(); })];
  return { el: workspace, destroy: () => { alive = false; offs.forEach((off) => off()); appearance.destroy(); systemMenu.destroy(); disposeLibrary(); workspace.remove(); } };
}
