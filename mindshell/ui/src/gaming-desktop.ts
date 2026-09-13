import { appearanceControls } from './appearance';
import * as bridge from './bridge';
import { every, h } from './dom';
import { renderGameLibrary } from './game-library';
import { openGaming } from './gaming';
import { icon } from './icons';
import { modeInfo, perfRefresh, perfSubscribe, perfSwitch } from './perf';
import { systemReadout } from './readout';
import { store } from './state';
import { systemControls } from './system-menu';
import type { PerfStatus } from './types';

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
  let alive = true;
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
  // The rail's readout is the Task Manager in miniature: the same shared
  // sampler, one call every three seconds for the whole card, and a way
  // through to the full thing. The top-bar line comes off the same sample.
  const paused = h('p', { class: 'gaming-meta', hidden: true }, 'Sampling paused while gaming');
  const readout = systemReadout({
    intervalMs: 3000,
    onSample: (v) => {
      const card = v.gpus[0];
      status.textContent = `${card?.util != null ? `GPU ${Math.round(card.util)}%${card.temp != null ? ` · ${Math.round(card.temp)}°` : ''}` : 'GPU unavailable'} / CPU ${Math.round(v.cpu.usage)}%`;
    },
  });
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
  const system = h('div', { class: 'gaming-rail-body' }, paused, readout.el, profileLabel, profile,
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
  // Only the padding: whether the workspace is on screen at all is the home
  // screen's business (see workspace.ts).
  const layout = () => {
    const pads = { top: 0, right: 0, bottom: 80, left: 0 };
    for (const p of store.state.layout.panels) if (p.output === '*' || p.output === output) pads[p.edge] = Math.max(pads[p.edge], p.size + p.margin * 2);
    for (const edge of ['top', 'right', 'bottom', 'left'] as const) workspace.style.setProperty(`--gaming-${edge}`, `${pads[edge]}px`);
  };
  // A game gets the machine to itself: the readout's timer already stops on a
  // desktop surface while one runs (see quiet.ts), so the card only has to say
  // why the numbers have stopped moving.
  const gameState = () => {
    paused.hidden = !store.state.game;
    readout.el.hidden = !!store.state.game;
    if (store.state.game) status.textContent = 'GameMode active / Desktop at rest';
  };
  layout(); renderLaunchers(); renderPerf(); gameState();
  const disposeLibrary = renderGameLibrary(library, toggleLibrary);
  const offs = [store.on('layout', layout), store.on('apps', renderLaunchers), store.on('game', gameState),
    perfSubscribe(workspace, (s) => { perf = s; renderPerf(); }),
    every(workspace, 15000, () => { if (!store.state.game && !document.hidden) void perfRefresh(); })];
  return { el: workspace, destroy: () => { alive = false; offs.forEach((off) => off()); readout.destroy(); appearance.destroy(); systemMenu.destroy(); disposeLibrary(); workspace.remove(); } };
}
