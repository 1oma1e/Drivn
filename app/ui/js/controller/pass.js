// Install pass and rollback: recap -> restore point question -> UAC -> one item at a time -> end (Spec - Installation).
import { t, shortName } from '../lib/i18n.js';
import { view, isOfficialTool, isSilentPackage } from '../model/selectors.js';
import { officialDialog } from './tools.js';

const INSTALLED = new Set(['ok', 'pending']);

export function passController({ store, backend, notify }) {
  let job = null;
  let followers = {}; // row id -> the row whose package updates it too
  let resumeTo = false; // pause state to restore when the stop dialog is dismissed

  const setPass = (patch) => store.set((s) => ({ pass: { ...s.pass, ...(typeof patch === 'function' ? patch(s.pass) : patch) } }));
  const setRows = (fn) => store.set((s) => ({ rows: fn({ ...s.rows }) }));
  const refresh = () => store.set((s) => ({ result: { ...s.result, items: backend.current() }, history: backend.history() }));

  // Each choice that changes the system goes through the UAC of Windows (asked by the installer).
  const elevate = (next) => {
    // R-SEC-01 (ADR-0036): before each real pass, the restore point is asked; "yes" is the default.
    if (next.kind === 'pass' && next.point == null && backend.real()) return store.set({ dialog: { kind: 'point', next } });
    store.set({ dialog: null });
    if (next.kind === 'pass') start(next.ids, null, next.point);
    else if (next.kind === 'resume') start(null, { ...store.get().interrupted, stop: next.stop });
    else rollback(next);
  };

  // ADR-0033, ADR-0023: a ticked Intel Wi-Fi or Java is not in Drivn's pass; the publisher's own installer opens
  // after it, one after the other (`after`). ADR-0034: NVIDIA's package runs silently inside the pass.
  async function ask(ids, direct = false) {
    const s = store.get();
    if (!ids.length || s.pass || s.interrupted) return;
    const items = s.result?.items ?? [];
    // One installer per package: NVIDIA's updates the card and its HD Audio together (the card's row first).
    const seen = new Map();
    const key = (x) => x.dev.tool ?? `${x.dec.offer?.src}|${x.dec.offer?.vv}`;
    const ticked = items.filter((x) => ids.includes(x.dev.id) && isOfficialTool(x))
      .sort((a, b) => /audio/i.test(a.dev.name) - /audio/i.test(b.dev.name));
    const official = ticked.filter((x) => !seen.has(key(x)) && seen.set(key(x), x.dev.id));
    // The other rows of a silent package (NVIDIA HD Audio) follow the card's row in the pass.
    followers = Object.fromEntries(ticked.filter((x) => isSilentPackage(x) && !official.includes(x))
      .map((x) => [x.dev.id, seen.get(key(x))]));
    const tools = official.filter((x) => !isSilentPackage(x)).map((x) => x.dev.id);
    const silent = official.filter(isSilentPackage).map((x) => x.dev.id);
    const rest = ids.filter((id) => !items.some((x) => x.dev.id === id && isOfficialTool(x))).concat(silent);
    if (rest.length) {
      // R-BUT-01: « Tout mettre à jour » starts at once, no recap (Windows still asks the UAC).
      if (direct) { store.set({ sel: null, after: tools }); return elevate({ kind: 'pass', ids: rest }); }
      return store.set({ dialog: { kind: 'recap', ids: rest, real: backend.real(), after: tools }, sel: null, after: tools });
    }
    store.set({ sel: null, after: tools.slice(1) });
    store.set({ dialog: await officialDialog(backend, items.find((x) => x.dev.id === tools[0])) });
  }

  /** `resume`: an interrupted pass taken up again (R-SEC-06); its finished items are shown as they were. */
  function start(ids, resume = null, point = true) {
    const real = resume ? !resume.sim : backend.real();
    const prior = resume ? resume.items : [];
    const along = resume ? {} : Object.fromEntries(Object.entries(followers).filter(([, lead]) => ids.includes(lead)));
    const all = resume ? [...prior.map((it) => it.id), resume.started, ...resume.rest] : [...ids, ...Object.keys(along)];
    const counted = new Set(prior.map((it) => it.id));
    if (resume && !resume.started) all.splice(prior.length, 1);
    store.set((s) => ({
      dialog: null, justDone: 0, reboot: [], interrupted: null, last: null, screen: s.screen === 'apps' ? 'apps' : 'drivers',
      pass: { ids: all, real, key: resume?.key ?? null, restore: { st: 'wait' }, done: prior.length, installedCount: prior.length,
        current: null, paused: false, stopReq: resume?.stop ?? null, log: resume ? [...resume.log] : [], at: Date.now() / 1000 },
      rows: { ...s.rows, ...Object.fromEntries(all.map((id) => [id, { st: 'queued' }])),
        ...Object.fromEntries(prior.map((it) => [it.id, { st: it.status ?? 'ok', real, why: it.why }])) },
    }));
    job = backend.install(resume ? resume.rest : ids, { point: point !== false }, function onEvent(ev, a, b, c) {
      if (ev === 'key') setPass({ key: a });
      else if (ev === 'log') setPass((p) => ({ log: [...p.log, a] }));
      else if (ev === 'step') setPass({ restore: { st: a, detail: b, since: c ?? null } });
      else if (ev === 'ask-restore') store.set({ dialog: { kind: 'restore', reason: a, existing: b ?? null } });
      else if (ev === 'refused' || ev === 'failed') gone(a, all);
      else if (ev === 'driver') {
        setRows((r) => ({ ...r, [a]: b }));
        if (b.st === 'run') setPass({ current: a });
        else {
          const first = !counted.has(a);
          counted.add(a);
          setPass((p) => ({ current: p.current === a ? null : p.current, done: p.done + (first ? 1 : 0), installedCount: p.installedCount + (first && INSTALLED.has(b.st) ? 1 : 0) }));
          if (b.st === 'ok' || b.st === 'rolled') refresh();
        }
        // The rows updated by the same package get its result.
        for (const [f, lead] of Object.entries(along)) {
          if (lead === a) onEvent('driver', f, b.st === 'run' ? { ...b, text: t('Avec le paquet de la carte…', 'With the card’s package…') } : { ...b, why: b.st === 'ok' ? t('mis à jour avec le paquet de la carte', 'updated with the card’s package') : b.why });
        }
      } else end(a, all);
    }, resume);
  }

  function end(pass, ids) {
    job = null;
    // Rows of items that were never started go back to their normal look, and so do the ones put back.
    setRows((r) => { for (const id of ids) if (!pass || r[id]?.st === 'queued' || r[id]?.st === 'rolled') delete r[id]; return r; });
    store.set({ pass: null });
    if (!pass) return notify(t('Annulé avant toute installation. Rien n’a été modifié.', 'Cancelled before any install. Nothing was changed.'));
    refresh();
    store.set({ justDone: pass.id, last: pass, dialog: { kind: 'end' } });
  }

  // The installer stopped (UAC refused, refused pass, crash): nothing more happens without the user.
  function gone(text, ids) {
    job = null;
    setRows((r) => { for (const id of ids) if (r[id]?.st === 'queued' || r[id]?.st === 'run') delete r[id]; return r; });
    store.set({ pass: null, dialog: null });
    notify(text);
    backend.loadPasses().then((p) => store.set({ history: p.history, interrupted: p.interrupted }));
  }

  async function rollback({ passId, index }) {
    // The UAC, then the installer: the user sees that something is running until it answers.
    store.set({ dialog: { kind: 'working', text: t('Retour arrière : Windows demande l’autorisation, puis remet l’ancien driver…', 'Rolling back: Windows asks for permission, then puts the old driver back…'), at: Date.now() / 1000 }, justDone: 0 });
    const it = await backend.rollback(passId, index);
    store.set({ dialog: null });
    if (!it) return notify(t('Retour arrière impossible : la sauvegarde n’existe plus.', 'Rollback failed: the backup no longer exists.'));
    if (it.error) { refresh(); return notify(it.error); }
    setRows((r) => { delete r[it.id]; return r; });
    refresh();
    const p = store.get().history.find((h) => h.id === passId);
    notify(t(`${shortName(it.name)} : revenu à ${it.from}`, `${shortName(it.name)}: back to ${it.from}`) + (p?.sim ? ' · simulation' : ''));
  }

  // R-SEC-08: the installer applies pause and stop between two drivers; here we only ask.
  function stop(mode) {
    if (!job) return;
    job.stop = mode;
    job.pause(false);
    setPass({ stopReq: mode, paused: false });
    store.set({ dialog: null });
  }

  // R-RBT-01: the pending reboot is only kept for a real pass where a driver asked for it.
  const keepReboot = () => {
    const p = store.get().last;
    store.set({ dialog: null, reboot: p && !p.sim ? p.reboot : [] });
  };

  return {
    actions: {
      'install-one'(d) { ask([d.id]); },
      'install-sel'() { ask(view(store.get()).selected.map((x) => x.dev.id)); },
      // R-BUT-01 (ADR-0034): everything ticked, one pass, no other question.
      'update-all'() {
        const s = store.get();
        const all = view(s).pickable;
        store.set({ picked: { ...s.picked, ...Object.fromEntries(all.map((x) => [x.dev.id, true])) } });
        ask(all.map((x) => x.dev.id), true);
      },
      'recap-go'() { elevate({ kind: 'pass', ids: store.get().dialog.ids }); },
      'point-yes'() { elevate({ ...store.get().dialog.next, point: true }); },
      'point-no'() { elevate({ ...store.get().dialog.next, point: false }); },
      'restore-yes'() { store.set({ dialog: null }); job?.reply(true); },
      'restore-no'() { store.set({ dialog: null }); job?.reply(false); },
      pause() { if (!job) return; const on = !store.get().pass.paused; job.pause(on); setPass({ paused: on }); },
      // The pass pauses while the user reads the choices (after the current driver, R-SEC-08).
      'stop-ask'() {
        if (!job) return;
        resumeTo = store.get().pass.paused;
        job.pause(true);
        setPass({ paused: true });
        store.set({ dialog: { kind: 'stop' } });
      },
      'stop-continue'() {
        job?.pause(resumeTo);
        setPass({ paused: resumeTo });
        store.set({ dialog: null });
      },
      'stop-keep'() { stop('keep'); },
      'stop-revert'() { stop('revert'); },
      'rollback-ask'(d) { store.set({ sel: null, dialog: { kind: 'rollback', passId: Number(d.pass), index: Number(d.index) } }); },
      'rollback-go'() { const d = store.get().dialog; elevate({ kind: 'rollback', passId: d.passId, index: d.index }); },

      journal(d) { store.set((s) => ({ dialog: { kind: 'journal', from: d.from, back: s.dialog?.kind === 'end' } })); },
      'journal-close'() { store.set((s) => ({ dialog: s.dialog.back ? { kind: 'end' } : null })); },
      'end-close'() { keepReboot(); },
      'reboot-later'() { keepReboot(); notify(t('Redémarrage en attente. Drivn ne redémarre jamais tout seul.', 'Restart pending. Drivn never restarts on its own.')); },

      // R-SEC-06: nothing is done without the user's choice, and each choice goes through the UAC.
      'cut-resume'() { elevate({ kind: 'resume', stop: null }); },
      'cut-keep'() { elevate({ kind: 'resume', stop: 'keep' }); },
      'cut-revert'() { elevate({ kind: 'resume', stop: 'revert' }); },
    },
  };
}
