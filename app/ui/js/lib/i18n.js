// French and English. Every text is written as a pair, t('Bonjour', 'Hello'): with two languages a
// pair can never be half-translated, and the text stays readable where it is shown.
// 'auto' follows the language of Windows (French if it is French, English otherwise).

let lang = 'fr';

/** Applies the setting ('auto', 'fr' or 'en'). */
export function setLang(choice) {
  const system = (navigator.languages?.[0] ?? navigator.language ?? 'en').toLowerCase();
  lang = choice === 'fr' || choice === 'en' ? choice : system.startsWith('fr') ? 'fr' : 'en';
  document.documentElement.lang = lang;
}

export const getLang = () => lang;

/** The text in the current language. */
export const t = (fr, en) => (lang === 'en' ? en : fr);

/** `n` and the noun that goes with it: plural(3, ['driver', 'drivers'], ['driver', 'drivers']). */
export const plural = (n, [frOne, frMany], [enOne, enMany]) => `${n} ${n > 1 || (lang === 'en' && n === 0) ? t(frMany, enMany) : t(frOne, enOne)}`;

/** "name : detail" in French, "name: detail" in English. */
export const sep = () => t(' : ', ': ');

/** Agreement only (no number): s(n) is 's' in French when n > 1. */
export const s = (n) => (n > 1 ? 's' : '');

const locale = () => (lang === 'en' ? 'en-GB' : 'fr-FR');

/** "2026-09-16" -> "16/09/2026" (fr) or "16 Sept 2026" (en); missing -> "—". */
export function shortDate(iso) {
  if (!iso) return '—';
  const d = new Date(`${iso.slice(0, 10)}T12:00:00`);
  if (Number.isNaN(d.getTime())) return iso;
  return d.toLocaleDateString(locale(), lang === 'en' ? { day: 'numeric', month: 'short', year: 'numeric' } : undefined);
}

/** A whole date of a Date object, in the current language. */
export const longDate = (d) => d.toLocaleDateString(locale());

export const hhmm = (d) => `${String(d.getHours()).padStart(2, '0')}:${String(d.getMinutes()).padStart(2, '0')}`;

/** "31/10 à 22:07" or "31/10 at 22:07", local time. */
export const dateTime = (d) => `${String(d.getDate()).padStart(2, '0')}/${String(d.getMonth() + 1).padStart(2, '0')} ${t('à', 'at')} ${hhmm(d)}`;

/** "45 s", "2 min 05 s": time spent on a step that gives no percentage, so it never looks frozen. */
export function elapsed(seconds) {
  const sec = Math.max(0, Math.floor(seconds));
  return sec < 60 ? `${sec} s` : `${Math.floor(sec / 60)} min ${String(sec % 60).padStart(2, '0')} s`;
}

/** "312 Mo" / "312 MB", "1,2 Go" / "1.2 GB". */
export function size(bytes) {
  const mb = bytes / (1024 * 1024);
  if (mb < 1024) return `${Math.round(mb)} ${t('Mo', 'MB')}`;
  return `${(mb / 1024).toLocaleString(locale(), { maximumFractionDigits: 1 })} ${t('Go', 'GB')}`;
}

/** Drops the (R) and (TM) marks Windows puts in device names. */
export const shortName = (n) => n.replace(/\((R|TM)\)/g, '').replace(/\s+/g, ' ').trim();

// Errors the Rust side sends as codes (app/src-tauri): worded here, in the user's language.
const ERRORS = {
  scan_first: () => t('Lance d’abord un scan.', 'Run a scan first.'),
  pass_running: () => t('Une installation est déjà en cours : attends qu’elle finisse.', 'An installation is already running: wait for it to finish.'),
  up_to_date: () => t('Drivn est déjà à jour.', 'Drivn is already up to date.'),
  no_reboot_pending: () => t('Aucune installation n’attend de redémarrage.', 'No installation is waiting for a restart.'),
  uac_refused: () => t('Autorisation administrateur refusée. Rien n’a été modifié.', 'Administrator permission refused. Nothing was changed.'),
  no_package: () => t('Aucun paquet officiel pour cet appareil.', 'No official package for this device.'),
  not_installed: () => t('Cet outil n’est pas installé sur ce PC.', 'This tool is not installed on this PC.'),
  winget_missing: () => t('winget n’est pas installé sur ce PC.', 'winget is not installed on this PC.'),
  winget_timeout: () => t('winget n’a pas répondu à temps.', 'winget did not answer in time.'),
  installer_missing: () => t('drivn-installer.exe est introuvable à côté de Drivn : réinstalle Drivn.', 'drivn-installer.exe is missing next to Drivn: reinstall Drivn.'),
};

/** An error from the backend in plain words; a technical one is kept as it is (it is a detail). */
export function errText(e) {
  const raw = String(e?.message ?? e ?? '');
  const code = raw.split(':')[0].trim();
  return ERRORS[code]?.() ?? raw;
}
