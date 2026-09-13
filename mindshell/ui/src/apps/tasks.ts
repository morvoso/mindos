// The Task Manager: what the machine is doing, and the means to stop it.
//
// Nine pages behind one sidebar. Overview is the dashboard; Processes is the
// table with the kill button; the rest go deep on one part of the machine
// each. Every page samples through the shared feed in monitor.ts, so a page
// left open in the background costs nothing extra, and switching pages does
// not start a second conversation with `/proc`.

import * as bridge from '../bridge';
import { h } from '../dom';
import { appFrame, pageHeader, setTitle } from './shared';
import { overviewPage, performancePage } from './tasks-overview';
import { processesPage } from './tasks-processes';
import { containersPage, networkPage, sensorsPage, servicesPage, storagePage, systemPage } from './tasks-system';

type PageFn = (el: HTMLElement, root: HTMLElement) => (() => void) | void;

const PAGES: { id: string; label: string; icon: string; group?: string; keywords?: string; render: PageFn }[] = [
  { id: 'overview', label: 'Overview', icon: 'gauge', keywords: 'dashboard summary at a glance load', render: overviewPage },
  { id: 'processes', label: 'Processes', icon: 'list', keywords: 'tasks kill end task pid memory cpu threads wine', render: processesPage },
  { id: 'performance', label: 'Performance', icon: 'pulse', group: 'Hardware', keywords: 'cpu cores threads clock frequency governor memory swap gpu vram graphics', render: performancePage },
  { id: 'storage', label: 'Storage', icon: 'hdd', group: 'Hardware', keywords: 'disk drive ssd nvme filesystem mount space free read write iops', render: storagePage },
  { id: 'network', label: 'Network', icon: 'ethernet', group: 'Hardware', keywords: 'wifi ethernet interface address ip throughput bandwidth vpn', render: networkPage },
  { id: 'sensors', label: 'Sensors', icon: 'bolt', group: 'Hardware', keywords: 'temperature heat fan rpm power watts thermal', render: sensorsPage },
  { id: 'containers', label: 'Containers', icon: 'docker', group: 'Services', keywords: 'docker podman images machines pods running', render: containersPage },
  { id: 'services', label: 'Services', icon: 'gear', group: 'Services', keywords: 'systemd units daemons failed running startup', render: servicesPage },
  { id: 'system', label: 'System', icon: 'system', group: 'Services', keywords: 'host kernel uptime motherboard bios firmware packages version', render: systemPage },
];

export function renderTasks(root: HTMLElement, page?: string): () => void {
  root.classList.add('app-tasks');
  const frame = appFrame(root, 'Task Manager', PAGES, show, {
    placeholder: 'Find a reading',
    empty: 'Nothing matches. Try “memory”, “docker” or “temperature”.',
    foot: 'Find a reading',
  });
  let dispose: (() => void) | void;
  function show(id: string): void {
    const p = PAGES.find((x) => x.id === id) ?? PAGES[0];
    if (dispose) dispose();
    frame.content.replaceChildren();
    frame.content.scrollTop = 0;
    frame.setActive(p.id);
    const el = h('div', { class: `page page-tasks-${p.id}` });
    frame.content.appendChild(el);
    dispose = p.render(el, root);
    setTitle(`${p.label} · Task Manager`);
  }
  show(page ?? 'overview');
  // Opened again from elsewhere (a desktop link, a notification): the one
  // window turns to the page asked for.
  const offOpen = bridge.on<{ page?: string }>('app.open', (p) => { if (p.page) show(p.page); });
  return () => {
    offOpen();
    if (dispose) dispose();
    frame.dispose();
  };
}

/** The page heading every Task Manager page wears, with its live sub-line. */
export function livePageHeader(title: string, sub: string): { el: HTMLElement; setSub(text: string): void } {
  const el = pageHeader(title, sub);
  const line = el.querySelector<HTMLElement>('.page-sub')!;
  return { el, setSub: (text) => (line.textContent = text) };
}
