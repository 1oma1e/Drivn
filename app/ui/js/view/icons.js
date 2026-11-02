// Line icons (24x24, one path each) and the device -> icon mapping.
import { raw } from '../lib/dom.js';

const PATHS = {
  game: 'M6 11h4M8 9v4M15 12h.01M18 10h.01M17.3 5H6.7a4 4 0 0 0-3.9 3.2l-1.1 5.5a3 3 0 0 0 5.2 2.6L8.5 15h7l1.6 1.3a3 3 0 0 0 5.2-2.6l-1.1-5.5A4 4 0 0 0 17.3 5z',
  wifi: 'M2 8.8a15 15 0 0 1 20 0M5 12.4a10 10 0 0 1 14 0M8.5 15.9a5 5 0 0 1 7 0M12 19.5h.01',
  wifioff: 'M2 2l20 20M8.5 15.9a5 5 0 0 1 7 0M12 19.5h.01M5 12.4a10 10 0 0 1 4-2.3M19 12.4a10 10 0 0 0-2.6-1.8M2 8.8a15 15 0 0 1 4.6-2.9M22 8.8a15 15 0 0 0-9-3.7',
  net: 'M4 4h16v10H4zM8 18h8M12 14v4M6 21h12',
  bt: 'M7 7l10 10-5 5V2l5 5L7 17',
  gpu: 'M2 6h17v11H2zM19 9h3v5h-3M6 17v3M10 17v3M8.5 11.5a2.5 2.5 0 1 0 5 0 2.5 2.5 0 0 0-5 0',
  monitor: 'M3 4h18v12H3zM8 20h8M12 16v4',
  audio: 'M11 5L6 9H2v6h4l5 4zM15.5 8.5a5 5 0 0 1 0 7M19 5a10 10 0 0 1 0 14',
  mic: 'M12 3a3 3 0 0 1 3 3v6a3 3 0 0 1-6 0V6a3 3 0 0 1 3-3zM5 11a7 7 0 0 0 14 0M12 18v3',
  sw: 'M4 8h4a2 2 0 1 1 4 0h4v4a2 2 0 1 1 0 4v4H4z',
  chip: 'M7 7h10v10H7zM9 2v3M15 2v3M9 19v3M15 19v3M19 9h3M19 15h3M2 9h3M2 15h3',
  cpu: 'M6 6h12v12H6zM9.5 9.5h5v5h-5zM9 2v4M15 2v4M9 18v4M15 18v4M18 9h4M18 15h4M2 9h4M2 15h4',
  mouse: 'M12 3a6 6 0 0 1 6 6v6a6 6 0 0 1-12 0V9a6 6 0 0 1 6-6zM12 7v3',
  kbd: 'M2 6h20v12H2zM6 10h.01M10 10h.01M14 10h.01M18 10h.01M7 14h10',
  pad: 'M6 8h12a4 4 0 0 1 4 4v1a4 4 0 0 1-7 2.6L14 14h-4l-1 1.6A4 4 0 0 1 2 13v-1a4 4 0 0 1 4-4zM7 11v3M5.5 12.5h3M16 11h.01M18 13h.01',
  disk: 'M4 6h16v12H4zM4 12h16M8 15h.01',
  usb: 'M12 2v14M8 6l4-4 4 4M7 11v2a2 2 0 0 0 2 2h3M17 9v4a2 2 0 0 1-2 2h-3M12 16a2.5 2.5 0 1 0 0 5 2.5 2.5 0 0 0 0-5z',
  cam: 'M3 7h12v10H3zM15 10l6-3v10l-6-3',
  shield: 'M12 3l8 3v6c0 4.5-3.4 8.3-8 9-4.6-.7-8-4.5-8-9V6zM8.5 12l2.5 2.5 4.5-5',
  undo: 'M9 14L4 9l5-5M4 9h10a6 6 0 0 1 0 12h-3',
  check: 'M20 6L9 17l-5-5',
  close: 'M6 6l12 12M18 6L6 18',
  arrow: 'M5 12h14M13 6l6 6-6 6',
  arrowup: 'M12 19V5M5 12l7-7 7 7',
  rescan: 'M21 12a9 9 0 1 1-2.6-6.4M21 4v5h-5',
  help: 'M12 3a9 9 0 1 0 0 18 9 9 0 0 0 0-18zM9.1 9a3 3 0 0 1 5.8 1c0 2-3 2.5-3 4M12 17h.01',
  search: 'M11 4a7 7 0 1 0 0 14 7 7 0 0 0 0-14zM21 21l-5-5',
  chevron: 'M9 6l6 6-6 6',
  scan: 'M12 3a9 9 0 1 0 9 9M12 7a5 5 0 1 0 5 5M12 12l7-7',
  list: 'M8 6h13M8 12h13M8 18h13M3 6h.01M3 12h.01M3 18h.01',
  history: 'M3 12a9 9 0 1 0 3-6.7L3 8M3 3v5h5M12 7v5l3 2',
  gear: 'M12 9a3 3 0 1 0 0 6 3 3 0 0 0 0-6zM12 2v3M12 19v3M2 12h3M19 12h3M4.9 4.9L7 7M17 17l2.1 2.1M4.9 19.1L7 17M17 7l2.1-2.1',
  pause: 'M6 4h4v16H6zM14 4h4v16h-4z',
  play: 'M7 4l13 8-13 8z',
};

const FILLED = new Set(['pause', 'play']);

/** Inline SVG icon. `w` is the stroke width. */
export function icon(name, size = 20, w = 2) {
  const paint = FILLED.has(name) ? 'fill="currentColor"' : `fill="none" stroke="currentColor" stroke-width="${w}" stroke-linecap="round" stroke-linejoin="round"`;
  return raw(`<svg width="${size}" height="${size}" viewBox="0 0 24 24" aria-hidden="true" ${paint}><path d="${PATHS[name]}"/></svg>`);
}

/** Picks the icon of a device from its family (`report.js`) and its name. */
export function deviceIcon(d) {
  const n = d.name.toLowerCase();
  switch (d.group) {
    case 'net': return /wi-?fi|wireless/.test(n) ? 'wifi' : 'net';
    case 'bt': return 'bt';
    case 'display': return /geforce|radeon|graphics/.test(n) ? 'gpu' : 'monitor';
    case 'audio': return /yeti|mic|voice/.test(n) ? 'mic' : 'audio';
    case 'storage': return 'disk';
    case 'usb': return 'usb';
    case 'input': return /keyboard/.test(n) ? 'kbd' : /gamepad|xbox/.test(n) ? 'pad' : 'mouse';
    case 'software':
    case 'apps': return 'sw';
    case 'camera': return 'cam';
    default: return /core\(tm\)|processor|ryzen/.test(n) ? 'cpu' : /trusted platform|firmware/.test(n) ? 'shield' : 'chip';
  }
}
