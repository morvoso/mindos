import * as bridge from '../bridge';
import { h } from '../dom';
import { store } from '../state';
import { icon } from '../icons';
import { card, notice, pageHeader, row } from './shared';

export function softwarePage(el: HTMLElement): void {
  const note = notice();
  const windowsReady = store.state.apps.some(app => app.id === 'mindos-win-open.desktop');
  const open = (label: string, cmd: string) => h('button', { class: 'btn accent', onclick: async () => {
    try { await bridge.call('shell.exec', { cmd }); }
    catch (error) { note.show(error instanceof Error ? error.message : String(error), 'error'); }
  } }, icon('external', 14), label);
  el.append(pageHeader('Software', 'Find what you want, review the changes, and install.'), note.el,
    card('Get more apps',
      row('Software manager', 'Search apps, see what is installed, remove apps, and install updates.', open('Open Software', 'gio launch /usr/share/applications/octopi.desktop')),
      h('p', { class: 'card-help' }, 'In Software, search for an app, select Install, then Apply. Review the list of changes before entering your password. Installed apps appear in Mind and the taskbar.')),
    card('Windows apps',
      row('Install a Windows program', 'Choose an .exe or .msi file. MindOS sets up its own space and opens the installer.', windowsReady ? open('Choose installer…', 'mindos-win-open') : open('Set up Windows apps', 'mindshell --app settings --page games')),
      h('p', { class: 'card-help' }, 'Use Steam for Steam games and Lutris for other game libraries. Windows apps have a small Windows badge. Compatibility depends on the app; some anti-cheat systems and driver-based software will not work.')),
    card('Graphics', h('p', { class: 'card-help' }, 'The MindOS installer detects AMD, Intel, NVIDIA and virtual graphics. It chooses the matching drivers and keeps integrated graphics on hybrid systems. Unsupported NVIDIA cards are identified before installation, with a compatibility option.'),
      open('Graphics guide', 'xdg-open /usr/share/doc/mindos/GRAPHICS.md')));
}
