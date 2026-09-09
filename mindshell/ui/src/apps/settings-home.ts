import * as bridge from '../bridge';
import { h } from '../dom';
import { icon } from '../icons';
import { modeInfo, perfRefresh, perfSubscribe } from '../perf';
import { store } from '../state';
import { card, notice, pageHeader, row } from './shared';

export function homePage(el: HTMLElement, navigate: (page: string) => void): () => void {
  const state = store.state;
  const note = notice();
  const mode = h('span', { class: 'pill' }, 'Checking…');
  const terminal = h('button', { class: 'btn', onclick: async () => {
    terminal.disabled = true;
    try { await bridge.call('shell.exec', { cmd: store.state.config.terminal || 'kitty' }); }
    catch (error) { note.show(bridge.reason(error), 'error'); }
    finally { terminal.disabled = false; }
  } }, icon('terminal', 15), 'Open terminal');
  const go = (page: string, title: string, text: string, symbol: string) => h('button', {
    class: 'settings-shortcut', onclick: () => navigate(page),
  }, h('span', { class: 'shortcut-icon' }, icon(symbol, 22)),
  h('span', { class: 'shortcut-copy' }, h('strong', {}, title), h('span', {}, text)), icon('chevron-right', 16));
  el.append(
    pageHeader('Overview'),
    note.el,
    card('System', row('Computer', null, h('strong', {}, state.host || 'MindOS')), row('Power profile', null, mode),
      h('div', { class: 'hero-actions' }, h('button', { class: 'btn primary', onclick: () => navigate('shell') }, 'Desktop settings'), terminal)),
    h('div', { class: 'settings-shortcuts' },
      go('performance', 'Performance', 'CPU, GPU and power profiles.', 'rocket'),
      go('wallpaper', 'Appearance', 'Wallpaper and desktop appearance.', 'image'),
      go('connections', 'Connections', 'Network, sound and devices.', 'wifi'),
      go('displays', 'Displays', 'Resolution, scaling and monitor arrangement.', 'display'),
      go('software', 'Software', 'Install and manage applications.', 'package'),
      go('updates', 'Updates', 'Check and apply software updates.', 'check')),
    card('Keyboard shortcuts',
      row('Find an app or ask the Mind', null, h('kbd', {}, 'Super + Space')),
      row('Open your terminal', null, h('kbd', {}, 'Super + Enter')),
      row('Switch window layouts', null, h('kbd', {}, 'Super + T')),
      row('Find any settings page', null, h('kbd', {}, 'Ctrl + K'))),
  );
  const off = perfSubscribe(el, (s) => {
    mode.textContent = s ? modeInfo(s.effective || s.mode).label : 'Unavailable';
    mode.classList.toggle('accent', !!s?.game);
    mode.title = s?.game ? 'GameMode is active' : 'Change this in Performance';
  });
  void perfRefresh();
  return off;
}

/** Reuse the system's device managers; the shell remains small and predictable. */
export function connectionsPage(el: HTMLElement): void {
  const note = notice();
  const launch = (label: string, cmd: string) => {
    const button = h('button', { class: 'btn', onclick: async () => {
      button.disabled = true;
      try { await bridge.call('shell.exec', { cmd }); }
      catch (error) { note.show(bridge.reason(error), 'error'); }
      finally { button.disabled = false; }
    } }, label, icon('external', 14));
    return button;
  };
  el.append(pageHeader('Connections & sound', 'Connect your network, pair your gear, and choose where sound goes.'), note.el,
    card('Network',
      row('Wi-Fi & Ethernet', 'Create and edit connections, passwords, DNS and routes.', launch('Network settings', 'nm-connection-editor')),
      h('p', { class: 'card-help' }, 'To join a nearby Wi-Fi network, open the NetworkManager icon in the system tray. It also handles network passwords.')),
    card('Devices', row('Bluetooth', 'Pair controllers, headphones, keyboards and other devices.', launch('Bluetooth settings', 'blueman-manager'))),
    card('Sound', row('Playback & microphone', 'Choose input and output devices, set levels per application, and test your microphone.', launch('Sound settings', 'pavucontrol'))));
}
