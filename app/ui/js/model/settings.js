// User settings, kept in the WebView's local storage (one small JSON).

const KEY = 'drivn.settings';
// showAll on by default: every device with a driver is listed. `lang`: 'auto' follows Windows.
const DEFAULTS = { motion: 'auto', showAbsent: false, showAll: true, lang: 'auto' };
const reduce = matchMedia('(prefers-reduced-motion: reduce)');

export function loadSettings() {
  try {
    return { ...DEFAULTS, ...JSON.parse(localStorage.getItem(KEY)) };
  } catch {
    return { ...DEFAULTS };
  }
}

export function saveSettings(s) {
  try { localStorage.setItem(KEY, JSON.stringify(s)); } catch { /* storage refused: settings last this session */ }
}

/** "auto" follows Windows ("Animation effects") and goes light on small CPUs (4 threads or less). */
export const resolveMotion = (m) =>
  m !== 'auto' ? m : reduce.matches || (navigator.hardwareConcurrency || 8) <= 4 ? 'lite' : 'full';

export const onSystemMotionChange = (fn) => reduce.addEventListener('change', fn);
