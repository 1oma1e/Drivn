// The one object the controllers talk to: the Rust side of the app (Tauri commands). The snapshot
// and the decisions come from the Rust engine (crates/core): nothing is decided here.
import { t, errText } from '../lib/i18n.js';
import { runLive, toPass, toInterrupted } from './livepass.js';
import { isSilentPackage } from './selectors.js';
import { toItems, searchStatus, presentCount, javaLatest, javaItems, biosItems, appItems, sourceErrors } from './report.js';

const invoke = (cmd, args) => window.__TAURI__.core.invoke(cmd, args);
/** Calls `cmd` with a progress channel: `onStep` gets every message. */
const withChannel = (cmd, args, onStep) => {
  const channel = new window.__TAURI__.core.Channel();
  channel.onmessage = onStep;
  return invoke(cmd, { ...args, onProgress: channel });
};
const CANCELLED = () => ({ ok: false, reason: t('Recherche annulée : rien n’a été comparé.', 'Search cancelled: nothing was compared.') });
const FAILED = (e) => ({ ok: false, reason: t(`Scan impossible : ${errText(e)}.`, `Scan failed: ${errText(e)}.`) });

export function createBackend() {
  let base = [];   // rows of the last scan
  let read = null; // snapshot devices read so far, kept if the search is cancelled
  let last = null; // what the last result was built from, to word it again in another language
  let passes = []; // real passes of this PC, read from their journals
  let mode = 'simulation'; // what the next pass will be

  const groupOf = (id) => base.find((x) => x.dev.id === id)?.dev.group ?? 'system';
  const keep = (p) => { passes = [p, ...passes.filter((q) => q.key !== p.key)].sort((a, b) => b.at - a.at); };
  const info = (snapshot) => ({ total: presentCount(snapshot), devices: toItems(snapshot, null, CANCELLED()).map((x) => x.dev) });

  /** Rows and banners of a scan. `wu`: a function, so a language change words it again. */
  function build(snapshot, report, wu, games = [], apps = [], appsError = null) {
    last = { snapshot, report, wu, games, apps, appsError };
    read = snapshot;
    const status = wu();
    const java = javaLatest(snapshot);
    // Java listed by winget is updated by winget, silently: the Java Update row is then not shown.
    const byWinget = apps.some((a) => a.id === 'Oracle.JavaRuntimeEnvironment');
    base = [...toItems(snapshot, report, status), ...(byWinget ? [] : javaItems(games, java)), ...biosItems(snapshot), ...appItems(apps)];
    const sources = [...sourceErrors(snapshot), ...(appsError ? [{ name: 'winget', why: errText(appsError) }] : [])];
    return { wu: status, games, java, sources, items: base, info: info(snapshot) };
  }

  return {
    /** Emits ('devices', info), ('read', pct, current), ('search'), ('vendors', progress), ('apps'), ('done', result). */
    scan(emit) {
      const job = { dead: false, kill() { job.dead = true; } };
      const out = (...a) => { if (!job.dead) emit(...a); };
      read = null;
      out('read', 0);
      withChannel('scan', {}, (msg) => {
        if (msg.event === 'reading') return out('read', msg.data.percent, msg.data.current);
        if (msg.event === 'vendors') return out('vendors', msg.data);
        if (msg.event === 'apps') return out('apps');
        if (msg.event !== 'hardware_read') return;
        read = { devices: msg.data.devices, windows_update: null };
        out('devices', info(read));
        out('search');
      })
        .then((r) => out('done', build(r.snapshot, r.report, () => searchStatus(r.snapshot), r.games, r.apps, r.apps_error ?? null)))
        .catch((e) => out('done', build(read ?? { devices: [] }, null, () => FAILED(e))));
      return job;
    },
    /** The hardware read so far, nothing compared: what is kept when the search is cancelled. */
    cancelled: () => build(read ?? { devices: [] }, null, CANCELLED),
    /** The last result, worded again in the current language. */
    relabel: () => (last ? build(last.snapshot, last.report, last.wu, last.games, last.apps, last.appsError) : null),
    current: () => base,

    /** "real" where the setup (or an administrator) turned real mode on (R-TST-01). */
    real: () => mode === 'real',
    install(ids, { point }, emit, resume) {
      // R-SEC-09 (ADR-0034): NVIDIA's package is fetched, then run silently by the installer.
      const packages = base.filter((x) => isSilentPackage(x) && ids.includes(x.dev.id)).map((x) => x.dev.id);
      const fetchPackage = (deviceId, onStep) => withChannel('package_fetch', { deviceId }, onStep);
      return runLive(invoke, { ids, resume, groupOf, packages, fetchPackage, point }, (ev, a, b) => {
        if (ev === 'end') keep(a);
        emit(ev, a, b);
      });
    },
    /** Brings back one driver: a UAC, then the installer. Resolves to the item, or { error }. */
    async rollback(passId, index) {
      const pass = passes.find((p) => p.id === passId);
      const item = pass?.items[index];
      if (!item) return null;
      try {
        const st = await invoke('pass_rollback', { passId: pass.key, n: item.n });
        const after = toPass(st, groupOf);
        keep(after);
        const now = after.items.find((it) => it.n === item.n);
        // Refused or failed: the last journal line (the end of this rollback) says why.
        if (now?.status === 'rolled') return now;
        return { error: st.error ? t(`Retour arrière refusé par l’installeur (${st.error}).`, `Rollback refused by the installer (${st.error}).`) : after.log.at(-1)?.text ?? t('Retour arrière impossible.', 'Rollback failed.') };
      } catch (e) {
        return { error: String(e) === 'uac_refused' ? errText(e) : t(`Retour arrière impossible : ${errText(e)}`, `Rollback failed: ${errText(e)}`) };
      }
    },
    history: () => passes,
    /** The passes of this PC, the interrupted one (R-SEC-06) and the restart that waits (R-RBT-01). */
    async loadPasses() {
      mode = await invoke('install_mode').catch(() => 'simulation');
      const list = await invoke('passes_list').catch(() => []);
      const valid = list.filter((st) => st.view);
      passes = valid.map((st) => toPass(st, groupOf)).filter((p) => !p.interrupted);
      const cut = valid.find((st) => (st.view.ended_at == null || st.view.reverting != null) && !st.running);
      const reboot = valid.filter((st) => st.reboot_pending).flatMap((st) => st.reboot);
      return { history: passes, interrupted: cut ? toInterrupted(cut, groupOf) : null, reboot };
    },
    rebootNow: () => invoke('reboot_now'),
    openProtection: () => invoke('open_protection'),
    // Updates of Drivn itself from GitHub Releases, signed with the author's key (ADR-0027).
    updateCheck: () => invoke('update_check'),
    updateInstall: (onStep) => withChannel('update_install', {}, onStep),
    // Official tools (R-SRC-10) and the restore point made before them (R-SEC-09).
    toolCheck: (kind) => invoke('tool_check', { kind }),
    toolLaunch: (kind) => invoke('tool_launch', { kind }),
    toolPage: (kind) => invoke('tool_page', { kind }),
    /** Read only: the link winget downloads this application from. */
    appSource: (id) => invoke('app_source', { id }),
    /** The publisher's installer (ADR-0030, ADR-0033): download, checks, launch; `onStep` gets each step. */
    packageInstall: (deviceId, onStep) => withChannel('package_install', { deviceId }, onStep),
    /** Runs a restore-point pass and waits for its answer: { ok, seq } or { ok: false, why }. */
    async restorePointAlone() {
      let started;
      try { started = await invoke('restore_point_start'); } catch (e) {
        return { ok: false, why: errText(e) };
      }
      if (started.simulation) return { simulation: true };
      for (let i = 0; i < 400; i++) {
        await new Promise((r) => setTimeout(r, 1500));
        const st = await invoke('pass_status', { passId: started.pass_id }).catch(() => null);
        const restore = st?.view?.restore;
        if (restore?.state === 'created') return { ok: true, seq: restore.seq };
        if (restore?.state === 'failed') {
          // The pass waits for an answer (R-SEC-01): it is stopped, nothing else was planned.
          await invoke('pass_control', { passId: started.pass_id, word: 'cancel_keep' }).catch(() => {});
          return { ok: false, why: restore.reason };
        }
        if (st && st.exit != null) return { ok: false, why: st.error ?? t('le point n’a pas été créé', 'the point was not created') };
      }
      return { ok: false, why: t('pas de réponse de Windows', 'no answer from Windows') };
    },
  };
}
