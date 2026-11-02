// Scan flow: launch animation, the scan steps, the verdict, then the Drivers screen.
import { t, hhmm } from '../lib/i18n.js';
import { isUpdate } from '../model/selectors.js';

export function scanController({ store, backend, notify }) {
  let job = null;
  let timer = 0;
  let vendorsAt = null;

  const setScan = (patch) => store.set((s) => ({ scan: { ...s.scan, ...patch } }));
  const later = (ms, fn) => { clearTimeout(timer); timer = setTimeout(fn, ms); };

  function run() {
    // `since`: start of the step that gives no percentage (Windows Update, winget), for its timer.
    store.set({ screen: 'scan', scan: { phase: 'read', p1: 0, peek: null, pct: 0, vendors: null, current: null, apps: false, since: null }, rows: {}, reboot: [], justDone: 0 });
    job = backend.scan((ev, p, extra) => {
      // Percent = devices read out of devices listed; 100 = hardware read (R-VRAI-01).
      if (ev === 'devices') store.set({ info: p });
      else if (ev === 'read') setScan(extra ? { p1: p, pct: p, current: extra } : { p1: p, pct: p });
      else if (ev === 'search') setScan({ phase: 'search', pct: 100, since: Date.now() / 1000 });
      else if (ev === 'vendors') { vendorsAt ??= Date.now(); setScan({ vendors: p, since: null }); }
      else if (ev === 'apps') setScan({ apps: true, since: Date.now() / 1000 });
      else finish(p);
    });
  }

  function finish(result) {
    // The publishers' step stays on screen a moment even when every answer came from the cache.
    const shown = vendorsAt ? Date.now() - vendorsAt : 1500;
    if (shown < 1500) { later(1500 - shown, () => finish(result)); return; }
    vendorsAt = null;
    job = null;
    const { items } = result;
    const tab = items.some((x) => isUpdate(x) || x.dec.state === 'announced') ? 'up' : items.some((x) => x.dec.state === 'up_to_date') ? 'ok' : 'unk';
    store.set({ info: result.info, result: { at: hhmm(new Date()), wu: result.wu, items, games: result.games ?? [], java: result.java ?? null, sources: result.sources }, tab, picked: {}, query: '' });
    setScan({ phase: 'found', pct: 100, since: null });
    later(2400, () => { const s = store.get(); if (s.screen === 'scan' && s.scan.phase === 'found') results(); });
  }

  function results() {
    clearTimeout(timer);
    store.set((s) => ({ screen: 'drivers', scan: { ...s.scan, phase: 'idle' } }));
  }

  function start() {
    const s = store.get();
    if (s.pass || (s.scan.phase !== 'idle' && s.scan.phase !== 'found')) return;
    store.set({ screen: 'scan', sel: null });
    setScan({ phase: 'launch' });
    later(850, run);
  }

  return {
    actions: {
      core() { if (store.get().scan.phase === 'found') results(); else start(); },
      rescan: start,
      results,
      'scan-stop'() {
        const reading = store.get().scan.phase === 'read';
        job?.kill();
        job = null;
        vendorsAt = null;
        if (reading) {
          // Nothing is kept from a half-read hardware list: the previous result stays.
          setScan({ phase: 'idle', since: null });
          notify(t('Scan arrêté. Rien n’a été modifié.', 'Scan stopped. Nothing was changed.'));
        } else finish(backend.cancelled()); // the hardware read is kept, nothing is compared
      },
    },
  };
}
