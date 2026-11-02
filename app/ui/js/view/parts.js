// Small view pieces shared by several screens: dialogs, buttons, help bubbles, busy lines.
import { html } from '../lib/dom.js';
import { t, elapsed } from '../lib/i18n.js';
import { icon } from './icons.js';

/** A dialog over a dimmed backdrop; `closeAct`: what a click outside (or Escape) does, none if empty. */
export const modal = (label, body, closeAct = '', wide = false) => html`<div class="overlay">
${closeAct ? html`<button type="button" class="backdrop" data-act="${closeAct}" aria-label="${t('Fermer', 'Close')}"></button>` : html`<span class="backdrop" aria-hidden="true"></span>`}
<div role="dialog" aria-modal="true" aria-label="${label}" class="glass dialog${wide ? ' wide' : ''}">${body}</div></div>`;

/** A row of buttons: [action, label, primary?]. */
export const buttons = (...list) => html`<div class="dbtns">${list.map(([act, label, primary]) => html`<button type="button" class="${primary ? 'btnp' : 'btng'}" data-act="${act}">${label}</button>`)}</div>`;

/** Dialog header: a coloured badge, a title and one line under it. */
export const dhead = (tone, ic, title, sub, tag = '') => html`<div class="dhead2"><span class="dbadge ${tone}">${icon(ic, 24, 2.2)}</span>
<div><strong class="dtitle">${title}${tag}</strong>${sub ? html`<span class="dsub">${sub}</span>` : ''}</div></div>`;

/**
 * A "?" that explains a word on hover or keyboard focus (R-VRAI-05: the detail stays out of the main
 * text). The bubble itself is drawn by main.js above everything, so no list or card can cut it.
 */
export const tip = (text) => html`<span class="tip" tabindex="0" role="note" aria-label="${text}" data-tip="${text}"><i aria-hidden="true">?</i></span>`;

/**
 * Something is running and gives no percentage: a turning ring, what it does, and the time spent,
 * so the screen never looks frozen. `since`: start, in seconds since 1970.
 */
export const busy = (text, since = null) => html`<span class="busyline" role="status"><span class="spinner" aria-hidden="true"></span><span>${text}${since != null ? html` <span class="mono dim">· ${elapsed(Date.now() / 1000 - since)}</span>` : ''}</span></span>`;

/** A progress bar: `pct` from 0 to 100, or null when the total is unknown (it then slides). */
export const bar = (pct) => html`<span class="track${pct == null ? ' busy' : ''}" role="progressbar" aria-valuemin="0" aria-valuemax="100"${pct == null ? '' : html` aria-valuenow="${Math.round(pct)}"`}><span class="fill" style="width:${pct == null ? 0 : Math.min(100, pct).toFixed(1)}%"></span></span>`;

/** Words used on several screens, explained in one place. */
export const HELP = {
  restore: () => t('Une photo de Windows prise avant d’installer. Si quelque chose ne marche plus, Windows peut revenir à cette photo (Paramètres → Système → Protection du système). Drivn garde ses 5 derniers points et supprime le plus ancien des siens, jamais ceux des autres.',
    'A snapshot of Windows taken before installing. If something stops working, Windows can go back to it (Settings → System → System Protection). Drivn keeps its 5 latest points and deletes the oldest of its own, never anyone else’s.'),
  backup: () => t('Avant de remplacer un driver, Drivn en garde une copie. Le bouton « Revenir » remet cette copie, pour ce driver seul. Drivn garde les sauvegardes de ses 5 dernières installations.',
    'Before replacing a driver, Drivn keeps a copy of it. The “Revert” button puts that copy back, for that driver only. Drivn keeps the backups of its 5 latest installations.'),
  signature: () => t('Une signature numérique prouve qui a publié le fichier et qu’il n’a pas été modifié. Drivn refuse un fichier non signé ou signé par quelqu’un d’autre que prévu.',
    'A digital signature proves who published the file and that it was not changed. Drivn refuses a file that is unsigned or signed by someone else than expected.'),
  windowsUpdate: () => t('Le service de Microsoft qui fournit les mises à jour de Windows et des drivers testés pour ton matériel. C’est la source la plus sûre.',
    'Microsoft’s service that delivers Windows updates and drivers tested for your hardware. It is the safest source.'),
  fragile: () => t('Un site officiel du fabricant, mais qui n’est pas prévu pour des programmes comme Drivn : il peut changer sans prévenir. Drivn vérifie quand même la signature de chaque fichier, et son empreinte quand le site la publie.',
    'An official site of the maker, but not made for programs like Drivn: it can change without notice. Drivn still checks the signature of every file, and its hash when the site publishes one.'),
  winget: () => t('Le gestionnaire d’applications de Microsoft, intégré à Windows. Il télécharge chaque appli depuis son éditeur et vérifie son empreinte.',
    'Microsoft’s app manager, built into Windows. It downloads each app from its publisher and checks its hash.'),
  hardwareId: () => t('Le numéro qui identifie un composant. Un driver n’est proposé que s’il est fait pour ce numéro exact.',
    'The number that identifies a component. A driver is offered only if it is made for that exact number.'),
  inf: () => t('Le fichier d’installation d’un driver : il dit à Windows quels fichiers copier et pour quel matériel.',
    'The setup file of a driver: it tells Windows which files to copy and for which hardware.'),
  risky: () => t('Plus récent, mais Drivn ne peut pas garantir qu’il est aussi bien adapté que ton driver actuel (plus générique, ou d’un autre éditeur). Pas coché par défaut.',
    'Newer, but Drivn cannot be sure it fits as well as your current driver (more generic, or from another publisher). Not ticked by default.'),
  announced: () => t('Une version plus récente annoncée par le site du fabricant. Elle est confirmée en ouvrant le paquet officiel, juste avant d’installer.',
    'A newer version announced by the maker’s site. It is confirmed by opening the official package, right before installing.'),
  uac: () => t('Windows demande ton accord avant qu’un programme change le système. Seul l’installeur de Drivn le demande, jamais l’app elle-même.',
    'Windows asks for your consent before a program changes the system. Only Drivn’s installer asks for it, never the app itself.'),
  simulation: () => t('Drivn fait tout le chemin (lectures, vérifications) sans rien installer. Le mode réel est activé par le setup de Drivn.',
    'Drivn goes through every step (reads, checks) without installing anything. Real mode is turned on by Drivn’s setup.'),
};
