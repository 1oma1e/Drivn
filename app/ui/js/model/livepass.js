// A pass of the installer (Spec - Installation). The app writes the plan, Windows shows the UAC,
// drivn-installer.exe does the pass and writes its journal; this only reads the journal and words it.
// Every status shown comes from a journal line (R-VRAI-01).
import { t, hhmm, shortName, elapsed, plural, errText, sep } from '../lib/i18n.js';

// Once a second: enough to follow the pass, light on the PC while it installs.
const POLL_MS = 1000;

// Journal status -> row status of the screens.
const STATUS = {
  installed: 'ok', installed_pending_reboot: 'pending', not_selected_by_windows: 'not_selected', failed: 'failed',
  rolled_back_unsigned: 'unsigned', unsigned_not_reverted: 'unsigned_kept', skipped: 'skipped',
  // A simulated driver is never "installed": no version change, no rollback button (R-VRAI-01).
  simulated: 'sim', not_installed: 'skipped', unknown: 'unknown',
  reverted: 'rolled', reverted_pending_reboot: 'rolled',
};
const SKIP = {
  source: () => t('source pas autorisée', 'source not allowed'),
  offer_gone: () => t('Windows Update ne propose plus cette mise à jour', 'Windows Update no longer offers this update'),
  device_absent: () => t('appareil débranché', 'device unplugged'),
  id_mismatch: () => t('l’ID du matériel ne correspond plus', 'the hardware ID no longer matches'),
  driver_changed: () => t('le driver a changé depuis le scan', 'the driver changed since the scan'),
  no_longer_newer: () => t('n’est plus plus récent', 'is no longer newer'),
  risky_not_checked: () => t('à vérifier, pas coché', 'to check, not ticked'),
  windows_driver: () => t('driver de Windows, sauvegarde impossible', 'Windows driver, no backup possible'),
  shared_id: () => t('un autre appareil a le même ID : Windows le changerait aussi, sans sauvegarde', 'another device has the same ID: Windows would change it too, without a backup'),
  backup_failed: () => t('sauvegarde impossible', 'backup failed'),
  read_failed: () => t('appareil illisible', 'device unreadable'),
  download_failed: () => t('téléchargement impossible : rien n’a été installé', 'download failed: nothing was installed'),
  refused_hash: () => t('fichier différent de celui annoncé par la source : supprimé, rien installé', 'file differs from the one the source announced: deleted, nothing installed'),
  refused_signature: () => t('signature absente ou inattendue : refusé, rien installé', 'signature missing or unexpected: refused, nothing installed'),
  extract_failed: () => t('paquet illisible ou suspect : refusé, rien installé', 'package unreadable or suspicious: refused, nothing installed'),
  no_matching_inf: () => t('le paquet ne contient pas de driver pour cet appareil', 'the package has no driver for this device'),
  compatible_id_only: () => t('le driver est trop général : il toucherait d’autres appareils', 'the driver is too generic: it would touch other devices'),
  refused_catalog: () => t('le driver n’est pas certifié par Microsoft : refusé', 'the driver is not certified by Microsoft: refused'),
};
const REFUSAL = {
  changed: () => t('refusé : ce driver a changé depuis', 'refused: this driver changed since'),
  reboot_first: () => t('redémarre d’abord, puis reviens en arrière', 'restart first, then roll back'),
  inf_shared: () => t('refusé : ce driver sert aussi à un autre appareil', 'refused: this driver is also used by another device'),
  nothing_installed: () => t('rien à remettre', 'nothing to put back'),
  expired: () => t('sauvegarde supprimée (5 passes gardées)', 'backup deleted (5 passes kept)'),
};
// [text, stage]: the stage lights one of Check, Back up, Install, Signature.
const STEP = {
  verify: [() => t('Revérification', 'Checking again'), 0],
  backup: [() => t('Sauvegarde du driver actuel', 'Backing up the current driver'), 1],
  install: [() => t('Installation par Windows', 'Installing through Windows'), 2],
  reread: [() => t('Relecture du driver actif', 'Reading the active driver'), 2],
  signature: [() => t('Vérification de la signature', 'Checking the signature'), 3],
  force: [() => t('Installation forcée (driver plus récent et signé)', 'Forced install (newer, signed driver)'), 2],
};
// Windows makes one restore point per 24 h: the one it already has is older than the pass.
const EXISTING = (n) => t(`Windows ne crée qu’un point de restauration par 24 h, et le point n° ${n} a moins de 24 h. Il date d’avant cette passe : il permet aussi de tout remettre.`,
  `Windows makes one restore point per 24 h, and point #${n} is less than 24 h old. It dates from before this pass: it can also put everything back.`);
// R-SEC-09: Drivn has no rollback for a publisher's installer; the restore point covers it.
const PKG_WHY = (p) => (p.status === 'updated' ? t('installé par l’installeur officiel · retour arrière : point de restauration', 'installed by the official installer · rollback: restore point')
  : p.status === 'simulated' ? t('vérifié · simulé, rien d’installé', 'checked · simulated, nothing installed')
  : t(`échec de l’installeur officiel${p.detail ? ` (${p.detail})` : ''}`, `the official installer failed${p.detail ? ` (${p.detail})` : ''}`));
const PKG_ST = (p) => (p.status === 'updated' ? 'ok' : p.status === 'simulated' ? 'sim' : p.status === 'failed' ? 'failed' : 'run');
const APP_ST = (a) => (a.status === 'updated' ? 'ok' : a.status === 'simulated' ? 'sim' : a.status === 'failed' ? 'failed' : 'run');
const APP_WHY = (a) => (a.status === 'updated' ? t('mis à jour par winget', 'updated by winget')
  : a.status === 'simulated' ? t('simulé, rien d’installé', 'simulated, nothing installed')
  : t(`échec de winget${a.detail ? ` (${a.detail})` : ''}`, `winget failed${a.detail ? ` (${a.detail})` : ''}`));
const hex = (code) => `0x${(code >>> 0).toString(16).toUpperCase().padStart(8, '0')}`;
const nowSec = () => Date.now() / 1000;

/** What happened to one driver, in a few words. */
export function why(r) {
  if (r.status === 'skipped') return t(`sauté : ${SKIP[r.skip]?.() ?? 'revérification'}`, `skipped: ${SKIP[r.skip]?.() ?? 'check'}`);
  // R-SEC-02, R-SEC-04 (ADR-0034): said, never a reason to skip.
  const saved = r.backup ? t('sauvegardé · ', 'backed up · ') : r.no_backup ? t('sans sauvegarde Drivn · ', 'no Drivn backup · ') : '';
  const forced = r.forced ? t('forcé · ', 'forced · ') : '';
  const back = r.after_interruption ? t('relu après la coupure : ', 'read again after the cut: ') : '';
  switch (r.status) {
    case 'installed': return `${back}${saved}${forced}${t('installé', 'installed')} · ${t('signature', 'signature')} ✓${r.signer ? ` (${r.signer})` : ''}`;
    case 'installed_pending_reboot': return `${saved}${forced}${t('installé · actif après redémarrage', 'installed · active after a restart')}`;
    case 'not_selected_by_windows': return t('pas retenu : Windows garde un driver mieux classé', 'not taken: Windows keeps a better-ranked driver');
    case 'failed': return t('échec de Windows', 'Windows failed') + (r.hresult ? ` (code ${hex(r.hresult)})` : '') + (r.detail ? `${sep()}${r.detail}` : '');
    case 'rolled_back_unsigned': return r.detail ? t('remis comme avant : signature invérifiable', 'put back: signature cannot be checked') : t('remis comme avant : signature refusée', 'put back: signature refused');
    case 'unsigned_not_reverted': return t('signature refusée et retour arrière raté : utilise le point de restauration de la passe', 'signature refused and rollback failed: use the pass’s restore point');
    case 'simulated': return t('revérifié · simulé, rien d’installé', 'checked again · simulated, nothing installed');
    case 'not_installed': return back + t('l’ancien driver est toujours actif', 'the old driver is still active');
    case 'unknown': return back + t('ni l’ancien ni le nouveau driver : à relire', 'neither the old nor the new driver: read it again');
    case 'reverted': return t('remis comme avant', 'put back');
    case 'reverted_pending_reboot': return t('remis comme avant après redémarrage', 'put back after a restart');
    default: return '';
  }
}

function restoreLine(r) {
  if (r.state === 'created') return t(`Point de restauration créé (n° ${r.seq}), vérifié dans la liste de Windows`, `Restore point created (#${r.seq}), checked in Windows’ list`);
  if (r.state === 'simulated') return t('Point de restauration : pas créé (simulation)', 'Restore point: not created (simulation)');
  if (r.state === 'declined') return t('Point de restauration : pas créé, tu as dit non', 'Restore point: not created, you said no');
  return r.existing ? t(`Pas de nouveau point : ${EXISTING(r.existing)}`, `No new point: ${EXISTING(r.existing)}`) : t(`Point de restauration impossible : ${r.reason}`, `Restore point failed: ${r.reason}`);
}

/** One journal line as the « Journal » dialog shows it; null for the lines that add nothing. */
function words(l, nameOf) {
  const name = (n) => shortName(nameOf(n) ?? `Driver ${n}`);
  switch (l.event) {
    case 'pass_start': return t(`Passe ${l.pass_id} : ${plural(l.items, ['driver', 'drivers'], ['driver', 'drivers'])} dans le plan`, `Pass ${l.pass_id}: ${plural(l.items, ['driver', 'drivers'], ['driver', 'drivers'])} in the plan`) + (l.simulation ? ' · simulation' : '');
    case 'offers_searched': return l.error ? t(`Recherche Windows Update impossible : ${l.error}`, `Windows Update search failed: ${l.error}`)
      : t(`Recherche Windows Update par l’installeur : ${plural(l.count, ['offre', 'offres'], ['offer', 'offers'])}`, `Windows Update search by the installer: ${plural(l.count, ['offre', 'offres'], ['offer', 'offers'])}`);
    case 'restore_start': return t('Point de restauration demandé à Windows', 'Restore point asked to Windows');
    case 'restore_done': return restoreLine(l.restore);
    case 'waiting_restore_consent': return t('En attente de ta réponse : continuer sans point de restauration ?', 'Waiting for your answer: go on without a restore point?');
    case 'continue_no_restore': return t('La passe continue sans point de restauration', 'The pass goes on without a restore point');
    case 'invalid_order': return t('Ordre invalide ignoré', 'Invalid order ignored');
    case 'paused': return t('En pause, entre deux drivers', 'Paused, between two drivers');
    case 'resumed': return t('Reprise', 'Resumed');
    case 'stop_requested': return l.mode === 'revert' ? t('Arrêt demandé : tout remettre comme avant', 'Stop asked: put everything back') : t('Arrêt demandé : garder ce qui est installé', 'Stop asked: keep what is installed');
    case 'wait_timeout': return t('Pas de réponse pendant 24 h : arrêt, rien de plus n’est installé', 'No answer for 24 h: stopped, nothing more is installed');
    case 'item_start': return `${name(l.n)} (${l.old?.version ?? t('aucun', 'none')} → ${l.offer.version ?? '?'})${sep()}${t('commencé', 'started')}`;
    case 'step_end':
      if (l.step === 'backup' && l.ok && l.detail) return t(`${name(l.n)} : sauvegarde dans ${l.detail}`, `${name(l.n)}: backup in ${l.detail}`);
      if (l.step === 'signature') return t(`${name(l.n)} : signataire ${l.detail ?? 'inconnu'}`, `${name(l.n)}: signer ${l.detail ?? 'unknown'}`);
      return null;
    case 'item_end': return `${name(l.record.n)}${sep()}${why(l.record)}`;
    case 'revert_start': return `${name(l.n)}${sep()}${t('retour arrière…', 'rolling back…')}`;
    case 'revert_end':
      if (l.status) return `${name(l.n)}${sep()}${why({ status: l.status })}`;
      if (l.refusal) return `${name(l.n)}${sep()}${REFUSAL[l.refusal]?.() ?? l.refusal}`;
      return t(`${name(l.n)} : retour arrière raté (${l.detail ?? '?'}) · Restauration du système : Paramètres → Système → Protection du système`,
        `${name(l.n)}: rollback failed (${l.detail ?? '?'}) · System Restore: Settings → System → System Protection`);
    case 'restore_end':
      if (l.error) return t(`Point de restauration n° ${l.seq} : fin pas notée (${l.error})`, `Restore point #${l.seq}: end not recorded (${l.error})`);
      return l.cancelled ? t(`Point de restauration n° ${l.seq} supprimé : rien n’a été installé`, `Restore point #${l.seq} deleted: nothing was installed`)
        : t(`Point de restauration n° ${l.seq} gardé`, `Restore point #${l.seq} kept`);
    case 'resume': return t('Reprise de la passe coupée (autorisation administrateur donnée)', 'Cut pass resumed (administrator permission given)');
    case 'pass_end': return t('Fin des drivers', 'Drivers done');
    case 'app_start': return t(`${l.name} : mise à jour par winget…`, `${l.name}: updating with winget…`);
    case 'app_end': return `${l.id}${sep()}${APP_WHY(l)}`;
    case 'apps_retry': return t(`${plural(l.ns.length, ['appli occupée', 'applis occupées'], ['busy app', 'busy apps'])} : 2ᵉ essai dans ${l.seconds} s`, `${plural(l.ns.length, ['appli occupée', 'applis occupées'], ['busy app', 'busy apps'])}: 2nd try in ${l.seconds} s`);
    case 'package_start': return t(`${shortName(l.name)} : installeur officiel lancé en silencieux…`, `${shortName(l.name)}: official installer started silently…`);
    case 'package_end': return t(`Installeur officiel ${l.n} : ${PKG_WHY(l)}`, `Official installer ${l.n}: ${PKG_WHY(l)}`);
    case 'point_removed': return l.ok ? t(`Ancien point de restauration n° ${l.seq} supprimé (Drivn garde ses 5 derniers)`, `Old restore point #${l.seq} deleted (Drivn keeps its 5 latest)`)
      : t(`Point n° ${l.seq} : suppression ratée, réessayée plus tard (${l.error})`, `Point #${l.seq}: deletion failed, tried again later (${l.error})`);
    case 'pruned': return l.ok ? t('Sauvegardes de cette passe supprimées (Drivn garde ses 5 dernières passes)', 'Backups of this pass deleted (Drivn keeps its 5 latest passes)')
      : t(`Ménage raté, réessayé à la prochaine passe : ${l.error}`, `Clean-up failed, tried again at the next pass: ${l.error}`);
    default: return null;
  }
}

const nameIn = (st) => (n) => st.plan.find((p) => p.n === n)?.name ?? st.view?.items.find((i) => i.n === n)?.name;
const idIn = (st) => (n) => st.plan.find((p) => p.n === n)?.id ?? st.view?.items.find((i) => i.n === n)?.device_instance_id;
const at = (l) => hhmm(new Date(l.at * 1000));
const logOf = (st) => st.lines.map((l) => ({ t: at(l), text: words(l, nameIn(st)) })).filter((l) => l.text);
/** When the last line of `event` for item `n` was written (seconds), or now. */
const startedAt = (st, event, n) => [...st.lines].reverse().find((l) => l.event === event && (n == null || l.n === n))?.at ?? nowSec();

let seq = 1000;
const uiIds = {};

/** A pass object as the end dialog and the History show it. `groupOf(id)`: family of a device of the scan. */
export function toPass(st, groupOf) {
  const v = st.view;
  const restore = v.restore?.state === 'created' && !v.restore_cancelled;
  const end = [...st.lines].reverse().find((l) => l.event === 'pass_end');
  return {
    id: (uiIds[st.pass_id] ??= ++seq), key: st.pass_id, at: new Date(v.started_at * 1000), sim: v.simulation,
    restore, seq: restore ? v.restore.seq : null, existing: v.restore?.existing ?? null,
    declined: v.restore?.state === 'declined', restoreWhy: v.restore?.state === 'failed' ? v.restore.reason : null, reboot: st.reboot, left: end?.not_started ?? 0, stopped: v.stop,
    expired: v.pruned, interrupted: v.ended_at == null || v.reverting != null,
    items: v.items.map((r) => {
      const item = {
        id: r.device_instance_id, n: r.n, name: r.name ?? r.device_instance_id, group: groupOf(r.device_instance_id),
        from: r.old?.version ?? t('aucun', 'none'), to: r.new?.version ?? '?', inf: r.old?.inf ?? null,
        status: STATUS[r.status] ?? 'unknown', raw: r.status, why: why(r), backup: !!r.backup,
      };
      // R-SRC-04 after the restart: read again by the app; the rollback is offered either way.
      const check = (st.after_reboot ?? []).find((c) => c.n === r.n);
      if (check?.state === 'verified') Object.assign(item, { status: 'ok', why: t(`actif après redémarrage · signature ✓ (${check.signer})`, `active after the restart · signature ✓ (${check.signer})`) });
      else if (check?.state === 'unsigned') Object.assign(item, { status: 'ok', why: t('actif après redémarrage, mais signature refusée : reviens en arrière', 'active after the restart, but signature refused: roll it back'), unsigned: true });
      else if (check?.state === 'not_active') Object.assign(item, { why: t('toujours pas actif après le redémarrage', 'still not active after the restart') });
      return item;
    }).concat((v.packages ?? []).map((p) => ({
      id: p.id, n: null, name: p.name, group: groupOf(p.id), from: '', to: '', inf: null,
      status: PKG_ST(p) === 'run' ? 'unknown' : PKG_ST(p), raw: p.status, why: PKG_WHY(p), backup: false,
    }))).concat((v.apps ?? []).map((a) => ({
      id: `app:${a.id}`, n: null, name: a.name, group: 'apps', from: '', to: '', inf: null,
      status: APP_ST(a) === 'run' ? 'unknown' : APP_ST(a), raw: a.status, why: a.status === 'failed' ? APP_WHY(a) : 'winget', backup: false,
    }))),
    log: logOf(st),
  };
}

/** The cut pass of the start-up screen (R-SEC-06), from its journal. */
export function toInterrupted(st, groupOf) {
  const v = st.view;
  const p = toPass(st, groupOf);
  const id = idIn(st);
  const started = v.started ? id(v.started.n) : null;
  const done = Math.max(v.started?.n ?? 0, ...v.items.map((i) => i.n), 0);
  const rest = st.plan.filter((r) => r.n > done).map((r) => r.id);
  return { ...p, started, rest, lineCount: st.lines.length, names: { started: started && shortName(nameIn(st)(v.started.n) ?? ''), rest } };
}

/**
 * Runs one pass (or takes up a cut one) and emits ('key', id), ('log', line), ('step', status, detail),
 * ('ask-restore', reason), ('driver', id, row), ('end', pass), ('refused', message), ('failed', message).
 * The returned job: pause(on), stop = 'keep'|'revert', reply(yes), kill().
 */
export function runLive(invoke, { ids, resume, groupOf, packages = [], fetchPackage, point = true }, emit) {
  let passId = null;
  let timer = 0;
  let stopMode = null;
  const send = (word) => passId && invoke('pass_control', { passId, word }).catch(() => {});
  const job = {
    dead: false,
    // R-SEC-08: the installer reads these words only between two drivers.
    pause(on) { if (!stopMode) send(on ? 'pause' : 'run'); },
    get stop() { return stopMode; },
    set stop(mode) { stopMode = mode; if (mode) send(mode === 'revert' ? 'cancel_revert' : 'cancel_keep'); },
    reply(yes) { send(yes ? 'continue_no_restore' : 'cancel_keep'); },
    kill() { job.dead = true; clearTimeout(timer); },
  };
  const out = (...a) => { if (!job.dead) emit(...a); };

  let seen = 0;
  let restoreKey = '';
  let asked = false;
  const shown = {};
  /** Emits a row only when what it shows changed (the elapsed time counts as a change). */
  const row = (id, value) => {
    const key = JSON.stringify(value);
    if (shown[id] === key) return;
    shown[id] = key;
    out('driver', id, value);
  };

  function follow(st) {
    const v = st.view;
    if (!v) {
      if (st.exit != null) out('failed', t(`L’installeur s’est arrêté avant de commencer (${st.error ?? `code ${st.exit}`}).`, `The installer stopped before starting (${st.error ?? `code ${st.exit}`}).`));
      return st.exit == null;
    }
    const nameOf = nameIn(st);
    for (const l of st.lines.slice(seen)) {
      const text = words(l, nameOf);
      if (text) out('log', { t: at(l), text });
    }
    seen = st.lines.length;

    let r = ['wait'];
    if (v.restore?.state === 'created') r = ['ok', t(`n° ${v.restore.seq}`, `#${v.restore.seq}`)];
    else if (v.restore?.state === 'simulated') r = ['sim', t('pas créé (simulation)', 'not created (simulation)')];
    else if (v.restore?.state === 'declined') r = ['skip', t('aucun (tu as dit non)', 'none (you said no)')];
    else if (v.restore?.state === 'failed') r = v.no_restore_chosen ? ['skip', v.restore.existing ? t(`n° ${v.restore.existing} (déjà là)`, `#${v.restore.existing} (already there)`) : t('impossible, la passe continue', 'failed, the pass goes on')] : ['warn', t('impossible', 'failed')];
    else if (st.lines.some((l) => l.event === 'restore_start')) r = ['run', '', startedAt(st, 'restore_start')];
    if (r.slice(0, 2).join() !== restoreKey) { restoreKey = r.slice(0, 2).join(); out('step', ...r); }
    // R-SEC-01: nothing is installed until the user answers.
    if (v.waiting_consent && !asked) {
      asked = true;
      const e = v.restore?.existing;
      out('ask-restore', e ? EXISTING(e) : v.restore?.reason ?? t('Windows a refusé le point de restauration.', 'Windows refused the restore point.'), e ?? null);
    }

    const id = idIn(st);
    // Finished drivers first: the one that starts next is then the current one.
    for (const item of v.items) row(id(item.n), { st: STATUS[item.status] ?? 'unknown', real: !v.simulation, why: why(item) });
    // R-SEC-09: publishers' installers, after the drivers, in the same pass.
    for (const p of v.packages ?? []) {
      const s = PKG_ST(p);
      row(p.id, s === 'run'
        // NVIDIA's installer restarts the graphics driver: the screens go black for a moment.
        ? { st: 'run', busy: true, text: `${t('Installeur officiel en cours', 'Official installer running')} · ${elapsed(nowSec() - startedAt(st, 'package_start', p.n))}`,
          hint: t('Les écrans peuvent devenir noirs quelques secondes : c’est normal, ne touche à rien.', 'Screens may go black for a few seconds: this is normal, do not touch anything.'), stage: 2 }
        : { st: s, real: !v.simulation, why: PKG_WHY(p) });
    }
    // ADR-0034: applications, after the drivers, in the same pass.
    for (const a of v.apps ?? []) {
      const s = APP_ST(a);
      const retry = (v.retrying ?? []).includes(a.n);
      row(`app:${a.id}`, retry || s === 'run'
        ? { st: 'run', busy: true, stage: 2, text: retry
          ? `${t('2ᵉ essai', '2nd try')} · ${elapsed(nowSec() - startedAt(st, 'apps_retry'))}`
          : `${t('Mise à jour par winget', 'Updating with winget')} · ${elapsed(nowSec() - startedAt(st, 'app_start', a.n))}` }
        : { st: s, real: !v.simulation, why: APP_WHY(a) });
    }
    if (v.started) {
      const step = v.started.step ?? 'verify';
      const [text, stage] = STEP[step];
      const stages = v.simulation ? 3 : 4;
      // Windows Update can take minutes on one driver: the time spent says it is still going.
      const since = startedAt(st, step === 'verify' ? 'item_start' : 'step_start', v.started.n);
      row(id(v.started.n), { st: 'run', prog: Math.round(((stage + 1) * 100) / (stages + 1)), text: `${text()}… · ${elapsed(nowSec() - since)}`, stage });
    }
    if (v.ended_at != null && v.reverting == null && !st.running) { out('end', toPass(st, groupOf)); return false; }
    if (st.exit != null && v.ended_at == null) {
      out('failed', t(`L’installeur s’est arrêté pendant la passe (${st.error ?? `code ${st.exit}`}). Elle est reprise au prochain lancement.`, `The installer stopped during the pass (${st.error ?? `code ${st.exit}`}). It is taken up at the next start.`));
      return false;
    }
    return true;
  }

  (async () => {
    try {
      if (resume) {
        await invoke('pass_resume', { passId: resume.key, stop: resume.stop ?? null });
        passId = resume.key;
        // The lines already read before the cut are not shown twice.
        seen = resume.lineCount ?? 0;
      } else {
        // R-SEC-09, R-BUT-02: each package is downloaded and checked first; one refused never stops the rest.
        const sent = ids.filter((x) => !packages.includes(x));
        for (const x of packages) {
          try {
            const from = nowSec();
            await fetchPackage(x, (p) => row(x, { st: 'run', stage: 0, prog: 10, busy: p.step !== 'download',
              text: p.step === 'download' ? t(`Téléchargement du paquet officiel : ${p.mb} Mo · ${elapsed(nowSec() - from)}`, `Downloading the official package: ${p.mb} MB · ${elapsed(nowSec() - from)}`)
                : t('Vérification du paquet (empreinte et signature)…', 'Checking the package (hash and signature)…') }));
            sent.push(`pkg:${x}`);
            // Checked: it now waits for its turn, after the drivers.
            row(x, { st: 'queued' });
          } catch (e) {
            row(x, { st: 'failed', real: true, why: errText(e) });
          }
        }
        if (!sent.length) return out('failed', t('Rien à installer : aucun paquet n’a passé les vérifications.', 'Nothing to install: no package passed the checks.'));
        passId = (await invoke('install_start', { ids: sent, restorePoint: point })).pass_id;
      }
    } catch (e) {
      return String(e) === 'uac_refused' ? out('refused', errText(e)) : out('failed', t(`Installation impossible : ${errText(e)}`, `Installation failed: ${errText(e)}`));
    }
    out('key', passId);
    const tick = async () => {
      if (job.dead) return;
      let go = true;
      try { go = follow(await invoke('pass_status', { passId })); } catch (e) { out('failed', t(`Journal illisible : ${errText(e)}`, `Journal unreadable: ${errText(e)}`)); go = false; }
      if (go && !job.dead) timer = setTimeout(tick, POLL_MS);
    };
    tick();
  })();
  return job;
}
