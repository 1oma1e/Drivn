// Composition root: builds the model, wires the controllers to clicks, renders the views.
import { createStore } from './lib/store.js';
import { patch } from './lib/dom.js';
import { t, setLang } from './lib/i18n.js';
import { createBackend } from './model/backend.js';
import { loadSettings, resolveMotion, onSystemMotionChange } from './model/settings.js';
import { appController } from './controller/app.js';
import { scanController } from './controller/scan.js';
import { passController } from './controller/pass.js';
import { toolsController } from './controller/tools.js';
import { railView } from './view/rail.js';
import { scanView } from './view/scan.js';
import { driversView } from './view/drivers.js';
import { gamesView } from './view/games.js';
import { historyView } from './view/history.js';
import { settingsView } from './view/settings.js';
import { overlaysView } from './view/overlays.js';

const SCREENS = { scan: scanView, drivers: driversView, apps: driversView, games: gamesView, history: historyView, settings: settingsView };
const REGIONS = {
  rail: railView,
  main: (s) => (['drivers', 'apps', 'games'].includes(s.screen) && !s.result ? scanView : SCREENS[s.screen])(s),
  layer: overlaysView,
};

const settings = loadSettings();
setLang(settings.lang);
const backend = createBackend();

const initial = {
  screen: 'scan', info: { total: 0, devices: [] },
  scan: { phase: 'idle', p1: 0, peek: null, pct: 0 },
  result: null, rows: {}, picked: {}, tab: 'up', query: '', sel: null, techOpen: false,
  pass: null, interrupted: null, last: null, dialog: null, after: [], history: [], open: {}, reboot: [], justDone: 0, notice: null,
  settings,
};

// Each region is re-rendered only when its HTML changed, then patched in place.
const last = {};
const nodes = Object.fromEntries(Object.keys(REGIONS).map((k) => [k, document.getElementById(k)]));
// During a pass the app goes light (no blur, no looping animation): the installers, and NVIDIA's which
// restarts the graphics driver, need the machine more than the window does.
const motionOf = (s) => {
  const m = resolveMotion(s.settings.motion);
  return s.pass && m === 'full' ? 'lite' : m;
};

function render(s) {
  const motion = motionOf(s);
  if (document.documentElement.dataset.motion !== motion) document.documentElement.dataset.motion = motion;
  // The live journal follows its new lines, unless the user scrolled up to read an older one.
  const before = document.querySelector('.jlog[data-follow]');
  const stick = !before || before.scrollHeight - before.scrollTop - before.clientHeight < 32;
  for (const [k, viewFn] of Object.entries(REGIONS)) {
    const out = String(viewFn(s));
    if (out !== last[k]) { patch(nodes[k], out); last[k] = out; }
  }
  const log = document.querySelector('.jlog[data-follow]');
  if (log && stick) log.scrollTop = log.scrollHeight;
}

const store = createStore(initial, render);
const applyMotion = (m) => { document.documentElement.dataset.motion = store.get().pass ? motionOf(store.get()) : resolveMotion(m); };
applyMotion(settings.motion);
onSystemMotionChange(() => applyMotion(store.get().settings.motion));

let noticeId = 0;
let noticeTimer = 0;
function notify(text) {
  const id = ++noticeId;
  store.set({ notice: { id, text } });
  clearTimeout(noticeTimer);
  noticeTimer = setTimeout(() => { if (store.get().notice?.id === id) store.set({ notice: null }); }, 6000);
}

const ctx = { store, backend, notify };
const actions = {
  ...appController({ ...ctx, applyMotion }),
  ...scanController(ctx).actions,
  ...passController(ctx).actions,
  ...toolsController(ctx).actions,
};

// One listener per event type for the whole app: elements name their action in data-act.
document.addEventListener('click', (e) => {
  const el = e.target.closest('[data-act]');
  if (el && !el.disabled) actions[el.dataset.act]?.(el.dataset);
});
document.addEventListener('input', (e) => {
  const el = e.target.closest('[data-input]');
  if (el) actions[el.dataset.input]?.(el.value);
});
// Escape does what a click on the top backdrop does; the restore question has none (an answer is required).
document.addEventListener('keydown', (e) => {
  if (e.key !== 'Escape') return;
  const backdrops = nodes.layer.querySelectorAll('.backdrop[data-act]');
  if (backdrops.length) backdrops[backdrops.length - 1].click();
});

render(store.get());

// Help bubbles: one box above everything, placed next to the "?" and kept inside the window.
const bubble = Object.assign(document.createElement('div'), { className: 'tipbox', role: 'tooltip' });
bubble.setAttribute('aria-hidden', 'true');
document.body.append(bubble);
function showTip(el) {
  bubble.textContent = el.dataset.tip;
  bubble.classList.add('on');
  const r = el.getBoundingClientRect();
  const b = bubble.getBoundingClientRect();
  const left = Math.min(Math.max(8, r.left + r.width / 2 - b.width / 2), innerWidth - b.width - 8);
  const top = r.top - b.height - 8 >= 8 ? r.top - b.height - 8 : r.bottom + 8;
  bubble.style.transform = `translate(${Math.round(left)}px, ${Math.round(top)}px)`;
}
const hideTip = () => bubble.classList.remove('on');
document.addEventListener('mouseover', (e) => { const el = e.target.closest?.('.tip'); if (el) showTip(el); else hideTip(); });
document.addEventListener('focusin', (e) => { if (e.target.matches?.('.tip')) showTip(e.target); else hideTip(); });
document.addEventListener('scroll', hideTip, true);

// Steps without a percentage show the time spent: once a second, while one runs, the screen is
// drawn again so that time moves (and the user sees the app is not frozen).
setInterval(() => {
  const s = store.get();
  if (s.pass || s.scan.since || s.dialog?.at) store.set({ tick: Date.now() });
}, 1000);

// A newer Drivn on GitHub (ADR-0027): a button in the rail, never an install without a click.
backend.updateCheck().then((u) => { if (u) store.set({ update: u }); }).catch(() => {});

// The passes of this PC, read from their journals: History, a pass cut by a crash (R-SEC-06), a restart that waits.
backend.loadPasses().then((p) => {
  store.set((s) => ({ history: p.history, interrupted: p.interrupted, reboot: p.reboot.length ? p.reboot : s.reboot }));
  // The cut pass is shown on the Drivers screen, with its choices, once the PC is read.
  const bad = p.history.flatMap((h) => h.items.filter((it) => it.unsigned));
  if (p.interrupted) notify(t('Une installation a été coupée. Lance un scan : Drivn te propose de la reprendre ou de l’arrêter.', 'An installation was cut. Run a scan: Drivn offers to resume or stop it.'));
  else if (bad.length) notify(t(`${bad.map((it) => it.name).join(', ')} : signature refusée après le redémarrage. Historique → Revenir.`, `${bad.map((it) => it.name).join(', ')}: signature refused after the restart. History → Revert.`));
}).catch(() => {});
