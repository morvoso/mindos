// The Settings app: Mind, updates, performance, games, software, wallpaper,
// displays, the screen (screensaver and lock), the desktop shell and about.

import { h } from '../dom';
import { appFrame, setTitle } from './shared';
import { displaysPage } from './settings-displays';
import { gamesPage } from './settings-games';
import { mindPage } from './settings-mind';
import { performancePage } from './settings-performance';
import { screenPage } from './settings-screen';
import { updatesPage } from './settings-updates';
import { aboutPage, shellPage } from './settings-shell';
import { wallpaperPage } from './settings-wallpaper';
import { connectionsPage, homePage } from './settings-home';
import { inputPage } from './settings-input';
import { softwarePage } from './settings-software';

type PageFn = (el: HTMLElement, root: HTMLElement) => (() => void) | void;

const PAGES: { id: string; label: string; icon: string; group?: string; keywords?: string; render: PageFn }[] = [
  { id: 'home', label: 'Overview', icon: 'grid', render: () => undefined },
  { id: 'performance', label: 'Performance', icon: 'rocket', group: 'Gaming', keywords: 'cpu gpu power mode boost scheduler memory latency', render: performancePage },
  { id: 'games', label: 'Games', icon: 'gamepad', group: 'Gaming', keywords: 'steam proton dlss fsr xess upscaler windows wine install', render: gamesPage },
  { id: 'shell', label: 'Desktop', icon: 'layout', group: 'Personalize', keywords: 'windows tiles columns panels dock widgets shortcuts cursor pointer theme', render: shellPage },
  { id: 'wallpaper', label: 'Wallpaper', icon: 'image', group: 'Personalize', keywords: 'background picture image color', render: wallpaperPage },
  { id: 'displays', label: 'Displays', icon: 'display', group: 'Personalize', keywords: 'screen monitor refresh hz resolution scale vrr', render: displaysPage },
  { id: 'screen', label: 'Screen & lock', icon: 'moon', group: 'Personalize', keywords: 'idle sleep timeout screensaver password suspend', render: screenPage },
  { id: 'input', label: 'Keyboard & mouse', icon: 'keyboard', group: 'Personalize', keywords: 'input layout language repeat typing acceleration sensitivity speed left handed scroll', render: inputPage },
  { id: 'connections', label: 'Connections & sound', icon: 'wifi', group: 'System', keywords: 'network wi-fi wifi ethernet bluetooth audio volume microphone headphones controllers', render: connectionsPage },
  { id: 'mind', label: 'Mind', icon: 'mind', group: 'System', keywords: 'assistant ai model thinking', render: mindPage },
  { id: 'software', label: 'Software', icon: 'package', group: 'System', keywords: 'apps install remove octopi packages windows wine', render: softwarePage },
  { id: 'updates', label: 'Updates', icon: 'package', group: 'System', keywords: 'upgrade software packages health rollback recovery', render: updatesPage },
  { id: 'about', label: 'About', icon: 'info', group: 'System', keywords: 'version hardware information', render: aboutPage },
];

export function renderSettings(root: HTMLElement, page?: string): () => void {
  root.classList.add('app-settings');
  const frame = appFrame(root, 'Settings', PAGES, show);
  let dispose: (() => void) | void;
  function show(id: string): void {
    const p = PAGES.find((x) => x.id === id) ?? PAGES[0];
    if (dispose) dispose();
    frame.content.replaceChildren();
    frame.content.scrollTop = 0;
    frame.setActive(p.id);
    const el = h('div', { class: `page page-${p.id}` });
    frame.content.appendChild(el);
    dispose = p.id === 'home' ? homePage(el, show) : p.render(el, root);
    setTitle(`${p.label} · Settings`);
  }
  show(page ?? 'home');
  return () => { if (dispose) dispose(); frame.dispose(); };
}
