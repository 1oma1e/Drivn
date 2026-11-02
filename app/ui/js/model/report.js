// Turns what the Rust backend returns (snapshot + decisions of crates/core) into the rows the screens show.
// The decisions themselves are never recomputed here: this only maps and words them.

import { t, dateTime } from '../lib/i18n.js';

// Windows device class -> family of the app (a key; `groupLabel` words it).
const GROUPS = {
  Net: 'net', Bluetooth: 'bt', Display: 'display', Monitor: 'display',
  MEDIA: 'audio', AudioProcessingObject: 'audio', SCSIAdapter: 'storage', DiskDrive: 'storage', HDC: 'storage',
  USB: 'usb', USBDevice: 'usb', HIDClass: 'input', Keyboard: 'input', Mouse: 'input',
  System: 'system', Processor: 'system', Firmware: 'system', SecurityDevices: 'system', Sensor: 'system',
  SoftwareComponent: 'software', Image: 'camera', Camera: 'camera',
};
const GROUP_LABELS = {
  net: ['Réseau', 'Network'], bt: ['Bluetooth', 'Bluetooth'], display: ['Affichage', 'Display'], audio: ['Audio', 'Audio'],
  storage: ['Stockage', 'Storage'], usb: ['USB', 'USB'], input: ['Périphériques', 'Input devices'], system: ['Système', 'System'],
  software: ['Logiciels', 'Software'], camera: ['Caméras', 'Cameras'], apps: ['Applications', 'Apps'],
};
export const groupLabel = (key) => t(...(GROUP_LABELS[key] ?? GROUP_LABELS.system));

// Drivers built into Windows are listed only for these classes; the others (volumes, endpoints…) are noise.
const KEEP_INBOX = new Set(['DiskDrive', 'Monitor', 'Processor', 'Keyboard', 'Mouse', 'USB', 'SecurityDevices', 'Firmware']);

// What a device is for, in plain words (R-VRAI-05): by Windows class, a few named cases first.
const ROLE_BY_NAME = [
  [/management engine/i, () => t('Puce de gestion de la carte mère Intel (alimentation, sécurité). Rien de visible pour toi.', 'Management chip of the Intel motherboard (power, security). Nothing you can see.')],
  [/(rst|vmd|raid|nvme|sata|ahci)/i, () => t('Fait le lien entre Windows et tes disques. Un mauvais driver ici peut empêcher le PC de démarrer.', 'Links Windows to your disks. A bad driver here can stop the PC from starting.')],
  [/hd audio|high definition audio/i, () => t('Envoie le son par le câble de l’écran (HDMI ou DisplayPort).', 'Sends sound through the display cable (HDMI or DisplayPort).')],
  [/asio/i, () => t('Son à très faible latence, pour la musique et l’enregistrement.', 'Very low latency sound, for music and recording.')],
  [/wi-?fi|wireless|wlan/i, () => t('Connexion Wi-Fi.', 'Wi-Fi connection.')],
];
const ROLE = {
  Net: () => t('Carte réseau : relie ton PC à Internet (câble ou Wi-Fi).', 'Network adapter: connects your PC to the Internet (cable or Wi-Fi).'),
  Bluetooth: () => t('Bluetooth : manettes, casques, souris et claviers sans fil.', 'Bluetooth: wireless controllers, headsets, mice and keyboards.'),
  Display: () => t('Carte graphique : affiche l’image et fait tourner les jeux.', 'Graphics card: draws the picture and runs games.'),
  Monitor: () => t('Écran : résolution et couleurs.', 'Monitor: resolution and colours.'),
  MEDIA: () => t('Son : haut-parleurs, casque et micro.', 'Sound: speakers, headset and microphone.'),
  AudioProcessingObject: () => t('Effets sonores : égaliseur, son 3D, réduction de bruit.', 'Sound effects: equalizer, 3D sound, noise reduction.'),
  SCSIAdapter: () => t('Contrôleur de stockage : fait le lien entre Windows et tes disques.', 'Storage controller: links Windows to your disks.'),
  HDC: () => t('Contrôleur de stockage : fait le lien entre Windows et tes disques.', 'Storage controller: links Windows to your disks.'),
  DiskDrive: () => t('Disque : où sont rangés Windows et tes fichiers.', 'Disk: where Windows and your files are stored.'),
  USB: () => t('Ports USB.', 'USB ports.'),
  USBDevice: () => t('Appareil branché en USB.', 'Device plugged in by USB.'),
  HIDClass: () => t('Clavier, souris, manette ou boutons spéciaux.', 'Keyboard, mouse, controller or special buttons.'),
  Keyboard: () => t('Clavier.', 'Keyboard.'),
  Mouse: () => t('Souris ou pavé tactile.', 'Mouse or touchpad.'),
  System: () => t('Composant de la carte mère : aide Windows à piloter le matériel.', 'Motherboard component: helps Windows drive the hardware.'),
  Processor: () => t('Processeur.', 'Processor.'),
  Firmware: () => t('Micrologiciel de la carte mère (UEFI).', 'Motherboard firmware (UEFI).'),
  SecurityDevices: () => t('Puce de sécurité (TPM).', 'Security chip (TPM).'),
  Sensor: () => t('Capteur (lumière, mouvement, orientation).', 'Sensor (light, motion, orientation).'),
  SoftwareComponent: () => t('Logiciel livré avec un driver : réglages ou service en arrière-plan.', 'Software that comes with a driver: settings or a background service.'),
  Image: () => t('Caméra ou scanner.', 'Camera or scanner.'),
  Camera: () => t('Caméra.', 'Camera.'),
};
function role(d) {
  const named = ROLE_BY_NAME.find(([re]) => re.test(d.description ?? ''));
  return named ? named[1]() : ROLE[d.class]?.() ?? null;
}

const REASON = {
  absent: () => t('Appareil débranché.', 'Device unplugged.'),
  no_search: () => t('Pas de recherche Windows Update : rien n’a été comparé.', 'No Windows Update search: nothing was compared.'),
  never_searched: () => t('Windows n’a encore jamais cherché de mises à jour sur ce PC : vérifie en ligne.', 'Windows never searched for updates on this PC yet: check online.'),
  doubtful_offer: () => t('Une offre existe, mais impossible de la comparer honnêtement.', 'An offer exists, but it cannot be compared honestly.'),
  installed_unreadable: () => t('Version du driver installé illisible : impossible de comparer.', 'The installed driver’s version cannot be read: no comparison.'),
  no_driver: () => t('Aucun driver, et rien sur Windows Update.', 'No driver, and nothing on Windows Update.'),
};
const REJECTED = {
  unknown_version: () => t('Offre sans version lisible', 'Offer without a readable version'),
  unknown_date: (v) => t(`${v} : offre sans date`, `${v}: offer without a date`),
  not_newer: (v) => t(`${v} n’est pas plus récent`, `${v} is not newer`),
  contradictory: (v) => t(`${v} est plus haut mais plus ancien en date : contradictoire`, `${v} is higher but older in date: contradictory`),
  installed_unreadable: () => t('Driver installé illisible', 'Installed driver unreadable'),
  not_chosen: (v) => t(`${v} : une autre offre a été choisie`, `${v}: another offer was chosen`),
};

/** What the Windows Update step gave: ok, or why nothing could be compared (R-VRAI-04). */
export function searchStatus(snapshot) {
  const wu = snapshot.windows_update;
  if (!wu) return { ok: false, reason: REASON.no_search() };
  // ADR-0017: a cached list is only as fresh as the last search Windows itself made.
  const at = wu.last_search_utc ? new Date(wu.last_search_utc) : null;
  const when = wu.online === false ? (at ? t(`liste de Windows du ${dateTime(at)}`, `Windows list of ${dateTime(at)}`) : null) : t('vérifié en ligne', 'checked online');
  if (wu.online === false && !at) return { ok: false, online: false, reason: REASON.never_searched() };
  if (wu.error) return { ok: false, online: wu.online !== false, when, reason: t(`Windows Update n’a pas répondu (${wu.error}).`, `Windows Update did not answer (${wu.error}).`) };
  return { ok: true, online: wu.online !== false, when };
}

function toDevice(d) {
  const drv = d.driver;
  return {
    id: d.instance_id, name: d.description ?? d.instance_id, group: GROUPS[d.class] ?? 'system', role: role(d),
    hw: d.hardware_ids, compat: d.compatible_ids, absent: !d.present,
    inst: drv && {
      v: drv.version, d: drv.date, p: drv.provider, match: drv.matching_device_id, inf: drv.inf_path,
      signer: drv.signature.state === 'signed' ? drv.signature.signer : null,
    },
  };
}

/** NVIDIA's numbering of a Windows DriverVer (`32.0.16.1692` → `616.92`), like `nvidia_version` in Rust. */
export function nvRelease(v) {
  const digits = (v ?? '').split('.').slice(2).join('');
  if (digits.length < 5 || !/^\d+$/.test(digits)) return null;
  const last = digits.slice(-5);
  return `${Number(last.slice(0, 3))}.${last.slice(3)}`;
}

/**
 * The two versions of a row, in one numbering. NVIDIA announces its own (`621.01`): the card shows its
 * driver in it too, and a driver of the package with another numbering (HD Audio) shows the package.
 */
export function versions(old, o) {
  const cur = old ? old.v : t('aucun', 'none');
  if (o?.src !== 'nvidia' || !o.vv) return { cur, nw: o?.v };
  const rel = nvRelease(old?.v);
  return rel ? { cur: `${rel} (${old.v})`, nw: o.vv } : { cur, nw: t(`paquet ${o.vv}`, `package ${o.vv}`) };
}

/** The site a row is downloaded from, in a few words (the full link is in its sheet). */
export function fromWhere(x) {
  const o = x.dec.offer;
  if (!o) return null;
  if (x.dev.tool === 'winget') return t('via winget', 'via winget');
  if (x.dev.tool === 'java') return t('via Java Update (Oracle)', 'via Java Update (Oracle)');
  const host = o.url ? hostOf(o.url) : null;
  if (host) return t(`depuis ${host}`, `from ${host}`);
  // Without a link, a source other than Windows Update is already named by « annoncée par ».
  return o.src ? null : t('depuis Windows Update (Microsoft)', 'from Windows Update (Microsoft)');
}
export const hostOf = (url) => { try { return new URL(url).host; } catch { return null; } };

/** Names of the sources other than Windows Update (the allow-list of R-SRC-01). */
export const SOURCE = { ms_catalog: 'Microsoft Update Catalog', nvidia: 'NVIDIA', oracle: 'Oracle', asus: 'ASUS', intel: 'Intel', winget: 'winget' };
export const sourceName = (key) => SOURCE[key] ?? key;

function toDecision(x, wu) {
  if (!x) return { state: 'unknown', reason: wu.reason, offer: null, risk: null, rejected: [], unchecked: [], hints: [] };
  const c = x.chosen;
  return {
    state: x.state,
    risk: x.risk ?? null,
    reason: x.reason === 'search_failed' ? wu.reason : REASON[x.reason ?? x.state]?.() ?? '',
    // `src`: null for Windows Update. A version only announced by a source (R-VER-03) keeps `vv`,
    // the publisher's own numbering, for display.
    offer: c && {
      v: c.version ?? c.vendor_version, d: c.date, p: c.provider, hid: c.hardware_id,
      src: c.source ?? null, vv: c.vendor_version ?? null, pub: c.publisher ?? null,
      size: c.size_text ?? null, installable: c.installable !== false, url: c.source_url ?? null,
    },
    rejected: x.rejected.map((r) => REJECTED[r.reason](r.version ?? '?')),
    // R-VRAI-04: sources that could not check this device; hints like "the NVIDIA package may hold newer".
    unchecked: (x.unchecked_by ?? []).map(sourceName),
    hints: (x.hints ?? []).map((h) => ({ kind: h.kind, text: h.text ?? '' })),
  };
}

/** Sources that failed for this scan, with their reason (R-VRAI-04: said, never hidden). */
export function sourceErrors(snapshot) {
  return (snapshot.vendors ?? []).filter((r) => r.error).map((r) => ({ name: sourceName(r.source), why: r.error }));
}

/** Latest Java 8 update read at Oracle (`1.8.0_511` → 511), or null. */
export function javaLatest(snapshot) {
  const oracle = (snapshot.vendors ?? []).find((r) => r.source === 'oracle');
  return oracle ? { update: oracle.java8_latest_update ?? null, error: oracle.error ?? null } : null;
}

/** `8.0.5010.8` → 501: the update number of a Java 8 version, or null. */
export function java8Update(version) {
  const m = /^8\.0\.(\d+)0\.\d+$/.exec(version ?? '');
  return m ? Number(m[1]) : null;
}

const NO_DECISION = { risk: null, reason: '', rejected: [], unchecked: [], hints: [] };

/**
 * Java 8 in the Apps tab: a row "announced by Oracle" when Oracle's version file has a newer update.
 * Not a driver: Java Update, Oracle's own tool, installs it (ADR-0023).
 */
export function javaItems(games, java) {
  if (!java?.update) return [];
  const seen = new Set();
  return games.filter((g) => g.kind === 'java').flatMap((g) => {
    const update = java8Update(g.version);
    if (update == null || update >= java.update || seen.has(update)) return [];
    seen.add(update);
    const offer = { v: `8 Update ${java.update}`, vv: null, d: null, p: 'Oracle', hid: null, src: 'oracle', pub: null, size: null, installable: false };
    return [{
      dev: { id: `java8:${update}`, name: g.name, group: 'software', tool: 'java', hw: [], compat: [], absent: false,
        role: t('Fait tourner les jeux et programmes écrits en Java (Minecraft Java…). Ce n’est pas un driver.', 'Runs games and programs written in Java (Minecraft Java…). It is not a driver.'),
        inst: { v: `8 Update ${update}`, d: null, p: g.publisher ?? 'Oracle' } },
      dec: { ...NO_DECISION, state: 'announced', offer },
      minor: false,
    }];
  });
}

/**
 * The BIOS of the motherboard (ADR-0032): installed against the maker's newest, shown only.
 * Drivn never downloads nor installs a BIOS.
 */
export function biosItems(snapshot) {
  const found = (snapshot.vendors ?? []).find((r) => r.bios);
  if (!found) return [];
  const b = found.bios;
  const src = found.source;
  const state = b.newer === true ? 'announced' : b.newer === false ? 'up_to_date' : 'unknown';
  const offer = b.newer === true
    ? { v: b.latest, d: b.latest_date, p: sourceName(src), hid: null, src, vv: null, pub: null, size: null, installable: false }
    : null;
  return [{
    dev: { id: 'bios', name: `BIOS (${b.board})`, group: 'system', bios: true, hw: [], compat: [], absent: false,
      role: t('Le programme qui démarre ta carte mère. Drivn ne le met jamais à jour lui-même.', 'The program that starts your motherboard. Drivn never updates it itself.'),
      inst: { v: b.installed ?? t('inconnu', 'unknown'), d: b.installed_date, p: sourceName(src) } },
    dec: { ...NO_DECISION, state, offer,
      reason: state === 'unknown' ? t(`Version du BIOS impossible à comparer (dernière chez ${sourceName(src)} : ${b.latest}).`, `BIOS version cannot be compared (latest at ${sourceName(src)}: ${b.latest}).`) : '' },
    minor: false,
  }];
}

/**
 * Applications with a newer version, from winget (ADR-0034): one row each, updated by winget in
 * the same pass as the drivers. The ID `app:<winget id>` is what the pass sends back.
 */
export function appItems(apps) {
  return (apps ?? []).map((a) => ({
    dev: { id: `app:${a.id}`, name: a.name, group: 'apps', tool: 'winget', hw: [], compat: [], absent: false,
      role: t('Application installée sur ton PC.', 'App installed on your PC.'), inst: { v: a.version || t('inconnue', 'unknown'), d: null, p: a.id } },
    dec: { ...NO_DECISION, state: 'announced',
      offer: { v: a.available, vv: null, d: null, p: 'winget', hid: null, src: 'winget', pub: null, size: null, installable: false } },
    minor: false,
  }));
}

/** A device worth a row: anything Windows Update spoke about, a device in trouble, or a real component. */
function listed(d, dec) {
  if (dec.offer || dec.rejected.length || dec.unchecked.length || dec.hints.length || d.problem_code != null) return true;
  return !!(d.description && d.driver && GROUPS[d.class] && (d.driver.provider !== 'Microsoft' || KEEP_INBOX.has(d.class)));
}

/**
 * Rows for the screens. `report` is null when nothing was compared (cancelled or no search):
 * every device is then "unknown" with `wu.reason`.
 */
export function toItems(snapshot, report, wu = searchStatus(snapshot)) {
  const seen = new Set();
  const items = [];
  snapshot.devices.forEach((d, i) => {
    const dec = toDecision(report?.decisions[i], wu);
    // Devices with a built-in Windows driver and nothing to say are kept, flagged `minor`: the
    // "Tous les appareils" setting shows them. A device without any driver stays out.
    const minor = !listed(d, dec);
    if (minor && !(d.driver && d.description)) return;
    // The same component often appears several times (one per port or function): one driver, one row.
    const key = `${d.description}|${d.driver?.version}|${dec.state}|${dec.offer?.v}`;
    if (seen.has(key)) return;
    seen.add(key);
    items.push({ dev: toDevice(d), dec, minor });
  });
  return items;
}

export const presentCount = (snapshot) => snapshot.devices.filter((d) => d.present).length;
