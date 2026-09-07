// The Settings app: Mind, updates, performance, games, developer, wallpaper,
// displays, the desktop shell and about.

import { h } from '../dom';
import { appFrame, setTitle } from './shared';
import { devPage } from './settings-dev';
import { displaysPage } from './settings-displays';
import { gamesPage } from './settings-games';
import { mindPage } from './settings-mind';
import { performancePage } from './settings-performance';
import { updatesPage } from './settings-updates';
import { aboutPage, shellPage } from './settings-shell';
import { wallpaperPage } from './settings-wallpaper';

type PageFn = (el: HTMLElement, root: HTMLElement) => (() => void) | void;

const PAGES: { id: string; label: string; icon: string; render: PageFn }[] = [
  { id: 'mind', label: 'Mind', icon: 'mind', render: mindPage },
  { id: 'updates', label: 'Updates', icon: 'package', render: updatesPage },
  { id: 'performance', label: 'Performance', icon: 'rocket', render: performancePage },
  { id: 'games', label: 'Games', icon: 'gamepad', render: gamesPage },
  { id: 'developer', label: 'Developer', icon: 'code', render: devPage },
  { id: 'wallpaper', label: 'Wallpaper', icon: 'image', render: wallpaperPage },
  { id: 'displays', label: 'Displays', icon: 'display', render: displaysPage },
  { id: 'shell', label: 'Desktop', icon: 'layout', render: shellPage },
  { id: 'about', label: 'About', icon: 'info', render: aboutPage },
];

export function renderSettings(root: HTMLElement, page?: string): void {
  root.classList.add('app-settings');
  const frame = appFrame(root, 'SETTINGS', PAGES, show);
  let dispose: (() => void) | void;
  function show(id: string): void {
    const p = PAGES.find((x) => x.id === id) ?? PAGES[0];
    if (dispose) dispose();
    frame.content.replaceChildren();
    frame.content.scrollTop = 0;
    frame.setActive(p.id);
    const el = h('div', { class: `page page-${p.id}` });
    frame.content.appendChild(el);
    dispose = p.render(el, root);
    setTitle(`${p.label} · Settings`);
  }
  show(page ?? 'mind');
}
