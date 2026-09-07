// A small line-icon set (24 viewBox, stroked with currentColor). Everything
// the shell shows is here so no icon theme is needed for the shell itself.

const P: Record<string, string> = {
  grid: '<rect x="3.5" y="3.5" width="7" height="7"/><rect x="13.5" y="3.5" width="7" height="7"/><rect x="3.5" y="13.5" width="7" height="7"/><rect x="13.5" y="13.5" width="7" height="7"/>',
  search: '<circle cx="11" cy="11" r="6.5"/><path d="M20 20l-4.2-4.2"/>',
  power: '<path d="M12 3v8"/><path d="M6.6 6.6a7.6 7.6 0 1 0 10.8 0"/>',
  reboot: '<path d="M4.5 12a7.5 7.5 0 1 0 2.2-5.3"/><path d="M4.5 4v5h5"/>',
  suspend: '<path d="M20 14.5A8 8 0 0 1 9.5 4a8 8 0 1 0 10.5 10.5z"/>',
  logout: '<path d="M10 4H5v16h5"/><path d="M14 8l4 4-4 4"/><path d="M18 12H9"/>',
  wifi: '<path d="M2.5 8.8a14.5 14.5 0 0 1 19 0"/><path d="M6 12.3a9.5 9.5 0 0 1 12 0"/><path d="M9.5 15.7a4.5 4.5 0 0 1 5 0"/><circle cx="12" cy="19" r="1.1" fill="currentColor" stroke="none"/>',
  ethernet: '<path d="M4 10h16v6H4z"/><path d="M8 10V6h8v4"/><path d="M8 16v2M12 16v2M16 16v2"/>',
  offline: '<path d="M2.5 8.8a14.5 14.5 0 0 1 19 0"/><path d="M6 12.3a9.5 9.5 0 0 1 12 0"/><path d="M9.5 15.7a4.5 4.5 0 0 1 5 0"/><path d="M4 4l16 16"/>',
  'volume-mute': '<path d="M4 9.5v5h3.5l5 4v-13l-5 4z"/><path d="M17 9.5l4 5M21 9.5l-4 5"/>',
  'volume-low': '<path d="M4 9.5v5h3.5l5 4v-13l-5 4z"/><path d="M16 9.5a3.5 3.5 0 0 1 0 5"/>',
  'volume-high': '<path d="M4 9.5v5h3.5l5 4v-13l-5 4z"/><path d="M16 9.5a3.5 3.5 0 0 1 0 5"/><path d="M18.8 6.5a7.5 7.5 0 0 1 0 11"/>',
  battery: '<rect x="2.5" y="7.5" width="16" height="9" rx="1.5"/><path d="M21.5 10.5v3"/>',
  bolt: '<path d="M12.5 3L6 13h5l-.5 8L18 11h-5z" fill="currentColor" stroke="none"/>',
  gear: '<circle cx="12" cy="12" r="3.2"/><path d="M12 2.5v3M12 18.5v3M2.5 12h3M18.5 12h3M5.3 5.3l2.1 2.1M16.6 16.6l2.1 2.1M5.3 18.7l2.1-2.1M16.6 7.4l2.1-2.1"/>',
  x: '<path d="M6.5 6.5l11 11M17.5 6.5l-11 11"/>',
  plus: '<path d="M12 5.5v13M5.5 12h13"/>',
  minus: '<path d="M5.5 12h13"/>',
  'chevron-left': '<path d="M14.5 6l-6 6 6 6"/>',
  'chevron-right': '<path d="M9.5 6l6 6-6 6"/>',
  'chevron-down': '<path d="M6 9.5l6 6 6-6"/>',
  'chevron-up': '<path d="M6 14.5l6-6 6 6"/>',
  pin: '<path d="M9 3.5h6l-1 5.5 3 3v2H7v-2l3-3z"/><path d="M12 14v6.5"/>',
  mind: '<path d="M12 2.6l8.1 4.7v9.4L12 21.4l-8.1-4.7V7.3z"/><path d="M12 7.6l4 4.4-4 4.4-4-4.4z"/>',
  cpu: '<rect x="6" y="6" width="12" height="12"/><rect x="9.5" y="9.5" width="5" height="5"/><path d="M9 2.5V6M15 2.5V6M9 18v3.5M15 18v3.5M2.5 9H6M2.5 15H6M18 9h3.5M18 15h3.5"/>',
  memory: '<rect x="2.5" y="7.5" width="19" height="9" rx="1"/><path d="M6.5 11v2M10 11v2M14 11v2M17.5 11v2M6.5 16.5v3M10 16.5v3M14 16.5v3M17.5 16.5v3"/>',
  gpu: '<rect x="2.5" y="6" width="18" height="11" rx="1"/><circle cx="10" cy="11.5" r="3"/><path d="M2.5 17v3.5M6.5 17v3.5M20.5 9h1v5h-1"/>',
  clock: '<circle cx="12" cy="12" r="8.5"/><path d="M12 7.5V12l3 2"/>',
  calendar: '<rect x="3.5" y="5" width="17" height="15.5" rx="1"/><path d="M3.5 10h17M8 3v4M16 3v4"/>',
  edit: '<path d="M4 20h4L18.5 9.5l-4-4L4 16z"/><path d="M13 7l4 4"/>',
  check: '<path d="M5 12.5l4.5 4.5L19 7.5"/>',
  grip: '<circle cx="9" cy="6" r="1.3" fill="currentColor" stroke="none"/><circle cx="15" cy="6" r="1.3" fill="currentColor" stroke="none"/><circle cx="9" cy="12" r="1.3" fill="currentColor" stroke="none"/><circle cx="15" cy="12" r="1.3" fill="currentColor" stroke="none"/><circle cx="9" cy="18" r="1.3" fill="currentColor" stroke="none"/><circle cx="15" cy="18" r="1.3" fill="currentColor" stroke="none"/>',
  terminal: '<rect x="3" y="4.5" width="18" height="15" rx="1"/><path d="M7 9.5l3 2.5-3 2.5M12 15h5"/>',
  monitor: '<rect x="3" y="4.5" width="18" height="12" rx="1"/><path d="M8 20h8M12 16.5V20"/>',
  panel: '<rect x="3" y="4.5" width="18" height="15" rx="1"/><path d="M3 15.5h18"/>',
  note: '<path d="M5.5 3.5h9l4 4v13h-13z"/><path d="M14.5 3.5v4h4M8.5 12h7M8.5 15.5h7"/>',
  window: '<rect x="3" y="4.5" width="18" height="15" rx="1"/><path d="M3 9.5h18"/>',
  // Four tilted panes: the "this is a Windows program" badge on dock icons.
  winapp: '<path d="M3 6.2l7.4-1.1v6.6H3zM11.6 4.9L21 3.5v8.2h-9.4zM3 12.7h7.4v6.6L3 18.2zM11.6 12.7H21v8.2l-9.4-1.4z" fill="currentColor" stroke="none"/>',
  apps: '<circle cx="6" cy="6" r="2"/><circle cx="12" cy="6" r="2"/><circle cx="18" cy="6" r="2"/><circle cx="6" cy="12" r="2"/><circle cx="12" cy="12" r="2"/><circle cx="18" cy="12" r="2"/><circle cx="6" cy="18" r="2"/><circle cx="12" cy="18" r="2"/><circle cx="18" cy="18" r="2"/>',
  gamepad: '<path d="M7 8h10a4.5 4.5 0 0 1 4.3 5.8l-1 3.2a2 2 0 0 1-3.3.7L15 15.5H9L7 17.7a2 2 0 0 1-3.3-.7l-1-3.2A4.5 4.5 0 0 1 7 8z"/><path d="M8 11v3M6.5 12.5h3M15.5 11.5h.01M17.5 13.5h.01"/>',
  code: '<path d="M8 8l-4 4 4 4M16 8l4 4-4 4M13.5 5l-3 14"/>',
  globe: '<circle cx="12" cy="12" r="8.5"/><path d="M3.5 12h17M12 3.5c3 3 3 14 0 17M12 3.5c-3 3-3 14 0 17"/>',
  music: '<path d="M9 18.5V6l11-2v12"/><circle cx="6.5" cy="18.5" r="2.5"/><circle cx="17.5" cy="16" r="2.5"/>',
  image: '<rect x="3.5" y="4.5" width="17" height="15" rx="1"/><circle cx="9" cy="10" r="1.5"/><path d="M20.5 15l-5-5-8 9"/>',
  office: '<path d="M6 3.5h9l4 4v13H6z"/><path d="M9 12h7M9 15.5h7"/>',
  system: '<circle cx="12" cy="12" r="3"/><path d="M12 3.5v3M12 17.5v3M3.5 12h3M17.5 12h3"/><circle cx="12" cy="12" r="8.5"/>',
  sliders: '<path d="M4 7h10M18 7h2M4 12h3M11 12h9M4 17h12M20 17h0"/><circle cx="15.5" cy="7" r="1.8"/><circle cx="8.5" cy="12" r="1.8"/><circle cx="17.5" cy="17" r="1.8"/>',
  box: '<path d="M12 3l8 4.5v9L12 21l-8-4.5v-9z"/><path d="M4 7.5l8 4.5 8-4.5M12 12v9"/>',
  layout: '<rect x="3.5" y="3.5" width="17" height="17" rx="1"/><path d="M3.5 9h17M9 9v11.5"/>',
  user: '<circle cx="12" cy="8.5" r="3.5"/><path d="M4.5 20a7.5 7.5 0 0 1 15 0"/>',
  info: '<circle cx="12" cy="12" r="8.5"/><path d="M12 11v5M12 8v.5"/>',
  drag: '<path d="M12 3.5v17M3.5 12h17M9 6.5l3-3 3 3M9 17.5l3 3 3-3M6.5 9l-3 3 3 3M17.5 9l3 3-3 3"/>',
  resize: '<path d="M20 14v6h-6M20 20l-7-7"/>',
  home: '<path d="M4 11l8-7 8 7v9h-5v-6h-6v6H4z"/>',
  refresh: '<path d="M4.5 12a7.5 7.5 0 1 0 2.2-5.3"/><path d="M4.5 4v5h5"/>',
  'eye-off': '<path d="M3 3l18 18M10 5.5A9 9 0 0 1 21 12a13 13 0 0 1-3 3.5M6.5 8A13 13 0 0 0 3 12a9 9 0 0 0 11.5 6"/>',
  desktop: '<rect x="3" y="4.5" width="18" height="12" rx="1"/><path d="M8 20h8M12 16.5V20"/><path d="M6.5 8h4"/>',
  // window layout modes
  'mode-floating': '<rect x="3.5" y="5" width="12" height="9.5" rx="1"/><rect x="9" y="10.5" width="11.5" height="9" rx="1" fill="var(--bg-0, #0a0d12)"/><path d="M3.5 8h12M9 13.5h11.5"/>',
  'mode-tiles': '<rect x="3.5" y="3.5" width="17" height="17" rx="1"/><path d="M12 3.5v17M12 12h8.5M16.25 12v8.5"/>',
  'mode-columns': '<rect x="3.5" y="3.5" width="17" height="17" rx="1"/><path d="M9.5 3.5v17M15.5 3.5v17"/>',
  // files / settings
  folder: '<path d="M3.5 6.5a1 1 0 0 1 1-1h5l2 2h8a1 1 0 0 1 1 1v9a1 1 0 0 1-1 1h-15a1 1 0 0 1-1-1z"/>',
  'folder-open': '<path d="M3.5 6.5a1 1 0 0 1 1-1h5l2 2h8a1 1 0 0 1 1 1v1.5"/><path d="M3.5 18.5l2.5-8h15.5l-2.5 8z"/>',
  file: '<path d="M6 3.5h8l4 4v13H6z"/><path d="M14 3.5v4h4"/>',
  trash: '<path d="M4.5 7h15M9.5 7V4.5h5V7M6.5 7l1 13h9l1-13"/><path d="M10 10.5v6M14 10.5v6"/>',
  copy: '<rect x="8.5" y="8.5" width="12" height="12" rx="1"/><path d="M15.5 8.5v-4a1 1 0 0 0-1-1h-10a1 1 0 0 0-1 1v10a1 1 0 0 0 1 1h4"/>',
  cut: '<circle cx="6.5" cy="17.5" r="2.5"/><circle cx="17.5" cy="17.5" r="2.5"/><path d="M8.5 15.7L18 4M15.5 15.7L6 4"/>',
  paste: '<rect x="5" y="5" width="14" height="16" rx="1"/><path d="M9 5V3.5h6V5M9 11h6M9 15h6"/>',
  download: '<path d="M12 3.5v12M7.5 11l4.5 4.5 4.5-4.5"/><path d="M4 17v3.5h16V17"/>',
  upload: '<path d="M12 15.5v-12M7.5 8L12 3.5 16.5 8"/><path d="M4 17v3.5h16V17"/>',
  list: '<path d="M8 6.5h12M8 12h12M8 17.5h12"/><circle cx="4.5" cy="6.5" r="1" fill="currentColor" stroke="none"/><circle cx="4.5" cy="12" r="1" fill="currentColor" stroke="none"/><circle cx="4.5" cy="17.5" r="1" fill="currentColor" stroke="none"/>',
  'grid-view': '<rect x="3.5" y="3.5" width="7" height="7"/><rect x="13.5" y="3.5" width="7" height="7"/><rect x="3.5" y="13.5" width="7" height="7"/><rect x="13.5" y="13.5" width="7" height="7"/>',
  'arrow-up': '<path d="M12 19.5v-15M5.5 11L12 4.5 18.5 11"/>',
  'arrow-left': '<path d="M19.5 12h-15M11 5.5L4.5 12l6.5 6.5"/>',
  'arrow-right': '<path d="M4.5 12h15M13 5.5l6.5 6.5-6.5 6.5"/>',
  external: '<path d="M14 4.5h5.5V10M19.5 4.5L11 13"/><path d="M17 13.5v6h-13v-13h6"/>',
  display: '<rect x="3" y="4.5" width="18" height="12" rx="1"/><path d="M8 20h8M12 16.5V20"/><path d="M6.5 13.5l3-3.5 2.5 2.5 3-4 2.5 5"/>',
  'check-circle': '<circle cx="12" cy="12" r="8.5"/><path d="M8 12.5l2.7 2.7L16.5 9.5"/>',
  warning: '<path d="M12 3.5l9 16h-18z"/><path d="M12 10v4M12 16.5v.5"/>',
  brain: '<path d="M12 2.6l8.1 4.7v9.4L12 21.4l-8.1-4.7V7.3z"/><path d="M12 7.6l4 4.4-4 4.4-4-4.4z"/>',
  sparkle: '<path d="M12 3.5l1.9 5.6 5.6 1.9-5.6 1.9L12 18.5l-1.9-5.6L4.5 11l5.6-1.9z"/>',
  hdd: '<rect x="3.5" y="9" width="17" height="10" rx="1"/><path d="M3.5 13h17M6.5 16h.01M9.5 16h.01M6.5 9L8 5h8l1.5 4"/>',
  usb: '<path d="M12 3.5v17M12 3.5l-2 2.5M12 3.5l2 2.5"/><circle cx="12" cy="19" r="1.5" fill="currentColor" stroke="none"/><path d="M12 15l-4-2.5V9M12 12.5l4-2V7.5"/><circle cx="8" cy="7.5" r="1.3"/><rect x="14.7" y="5" width="2.6" height="2.6"/>',
  eye: '<path d="M2.5 12s3.5-6 9.5-6 9.5 6 9.5 6-3.5 6-9.5 6-9.5-6-9.5-6z"/><circle cx="12" cy="12" r="2.7"/>',
  rename: '<path d="M4 20h4L18.5 9.5l-4-4L4 16z"/><path d="M13 7l4 4"/>',
  keyboard: '<rect x="2.5" y="6" width="19" height="12" rx="1"/><path d="M6 9.5h.01M9.5 9.5h.01M13 9.5h.01M16.5 9.5h.01M6 13h.01M9.5 13h.01M13 13h.01M16.5 13h.01M7.5 15.5h9"/>',
  star: '<path d="M12 3.5l2.6 5.5 6 .7-4.4 4.1 1.2 5.9L12 16.8l-5.4 2.9 1.2-5.9-4.4-4.1 6-.7z"/>',
  bell: '<path d="M6 16.5V11a6 6 0 0 1 12 0v5.5l1.5 2h-15z"/><path d="M10 20.5a2 2 0 0 0 4 0"/>',
  'bell-off': '<path d="M6 16.5V11a6 6 0 0 1 9.4-4.9M18 11v5.5l1.5 2H9"/><path d="M10 20.5a2 2 0 0 0 4 0"/><path d="M4 4l16 16"/>',
  rocket: '<path d="M12 3c3.5 2.5 5 6.5 4 11l-4 4-4-4C7 9.5 8.5 5.5 12 3z"/><path d="M8 13l-3 2 1 3M16 13l3 2-1 3"/><circle cx="12" cy="10" r="1.4"/>',
  gauge: '<path d="M4.5 16.5a8.5 8.5 0 1 1 15 0"/><path d="M12 15l4-5"/><circle cx="12" cy="15.5" r="1.3" fill="currentColor" stroke="none"/>',
  leaf: '<path d="M5 19c0-8 5-13 14-13-1 9-6 13-13 13z"/><path d="M5 19c3-4 6-7 10-9"/>',
  package: '<path d="M12 3l8 4.5v9L12 21l-8-4.5v-9z"/><path d="M4 7.5l8 4.5 8-4.5M12 12v9M8 5.3l8 4.5"/>',
  lock: '<rect x="4.5" y="10.5" width="15" height="10" rx="2.5"/><path d="M8 10.5V7.5a4 4 0 0 1 8 0v3"/>',
  shield: '<path d="M12 3l7.5 3v5.5c0 4.5-3 8-7.5 9.5-4.5-1.5-7.5-5-7.5-9.5V6z"/><path d="M9 12l2 2 4-4"/>',
  history: '<path d="M4.5 12a7.5 7.5 0 1 0 2.2-5.3"/><path d="M4.5 4v5h5"/><path d="M12 8v4l2.5 2"/>',
  pulse: '<path d="M3 12h4l2.5-6 4 12 2.5-6h5"/>',
  moon: '<path d="M20 14.5A8 8 0 0 1 9.5 4a8 8 0 1 0 10.5 10.5z"/>',
  swap: '<path d="M4 8h13l-3-3M20 16H7l3 3"/>',
  wrench: '<path d="M14.5 4.5a4.5 4.5 0 0 0-5.7 5.7L4 15v5h5l4.8-4.8a4.5 4.5 0 0 0 5.7-5.7l-3 3-2-2z"/>',
  docker: '<path d="M3 12h13a4 4 0 0 0 4-4 3 3 0 0 1 1 2.5c0 5-4 8-9 8-4 0-7-2-9-6.5z"/><path d="M6 12V9h3v3M10 12V9h3v3M10 8V5h3v3M14 12V9h3v3"/>',
  layers: '<path d="M12 3.5l8.5 4.5L12 12.5 3.5 8z"/><path d="M3.5 12l8.5 4.5 8.5-4.5M3.5 16l8.5 4.5 8.5-4.5"/>',
  'rotate': '<path d="M4.5 12a7.5 7.5 0 1 0 2.2-5.3"/><path d="M4.5 4v5h5"/>',
  'chevron-updown': '<path d="M8 9.5l4-4 4 4M8 14.5l4 4 4-4"/>',
};

export type IconName = keyof typeof P & string;

export function iconSvg(name: string, size = 18, cls = ''): string {
  const body = P[name] ?? P.box;
  return `<svg class="ic ${cls}" width="${size}" height="${size}" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">${body}</svg>`;
}

const tpl = document.createElement('template');

export function icon(name: string, size = 18, cls = ''): SVGSVGElement {
  tpl.innerHTML = iconSvg(name, size, cls);
  return tpl.content.firstElementChild!.cloneNode(true) as SVGSVGElement;
}

/** Battery glyph with a fill proportional to `percent` (and a bolt when charging). */
export function batteryIcon(percent: number, charging: boolean, size = 18): SVGSVGElement {
  const w = Math.max(0, Math.min(12.5, (12.5 * percent) / 100));
  tpl.innerHTML = `<svg class="ic" width="${size}" height="${size}" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.75" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">${P.battery}<rect x="4.25" y="9.25" width="${w}" height="5.5" fill="currentColor" stroke="none"/>${charging ? P.bolt : ''}</svg>`;
  return tpl.content.firstElementChild!.cloneNode(true) as SVGSVGElement;
}

/** A data: URL for a fallback application icon: a chamfered tile with an initial. */
export function letterIcon(text: string, hue: number): string {
  const ch = (text.trim()[0] ?? '?').toUpperCase();
  const svg = `<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 48 48'><path d='M6 0h42v42l-6 6H0V6z' fill='hsl(${hue} 45% 18%)'/><path d='M6 .5h41.5V42l-5.5 5.5H.5V6z' fill='none' stroke='hsl(${hue} 70% 55%)' stroke-opacity='.6'/><text x='24' y='31' font-family='Inter,sans-serif' font-weight='700' font-size='24' text-anchor='middle' fill='hsl(${hue} 90% 80%)'>${ch}</text></svg>`;
  return 'data:image/svg+xml;charset=utf-8,' + encodeURIComponent(svg);
}

export function hashHue(s: string): number {
  let h = 0;
  for (let i = 0; i < s.length; i++) h = (h * 31 + s.charCodeAt(i)) >>> 0;
  return h % 360;
}
