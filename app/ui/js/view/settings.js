// Settings screen. Nothing here can weaken security (no "install unsigned" option).
import { html } from '../lib/dom.js';
import { t, getLang } from '../lib/i18n.js';
import { resolveMotion } from '../model/settings.js';
import { tip, HELP } from './parts.js';

const MOTION = [['auto', () => t('Auto', 'Auto')], ['full', () => t('Complètes', 'Full')], ['lite', () => t('Légères', 'Light')]];
const LANGS = [['auto', () => t('Auto', 'Auto')], ['fr', () => 'Français'], ['en', () => 'English']];

const line = (k, v, tone = '', help = '') => html`<div class="sline"><span>${k}${help ? tip(help) : ''}</span><span class="sval ${tone}">${v}</span></div>`;
const seg = (label, list, value, act) => html`<span class="seg" role="radiogroup" aria-label="${label}">${list.map(([k, text]) => html`<button type="button" role="radio" aria-checked="${value === k}" class="${value === k ? 'on' : ''}" data-act="${act}" data-to="${k}">${text()}</button>`)}</span>`;

export function settingsView(s) {
  const { motion, showAbsent, showAll, lang } = s.settings;
  return html`<section class="page settings rise" data-key="settings">
<h1>${t('Réglages', 'Settings')}</h1>

<h2>${t('Langue', 'Language')}</h2>
<div class="sgroup">
<div class="sline"><span>${t('Langue de l’app', 'App language')}<small>${lang === 'auto' ? t(`Auto : suit la langue de Windows (${getLang() === 'fr' ? 'français' : 'anglais'})`, `Auto: follows the language of Windows (${getLang() === 'fr' ? 'French' : 'English'})`) : ''}</small></span>
${seg(t('Langue', 'Language'), LANGS, lang, 'lang')}</div>
</div>

<h2>${t('Sources', 'Sources')}</h2>
<div class="sgroup">
${line('Windows Update', t('toujours', 'always'), 'ok', HELP.windowsUpdate())}
${line('Microsoft Update Catalog, NVIDIA, ASUS, Oracle', t('sites officiels non documentés', 'official, undocumented sites'), '', HELP.fragile())}
${line(t('Applications (winget)', 'Apps (winget)'), t('toujours', 'always'), 'ok', HELP.winget())}
</div>

<h2>${t('Sécurité', 'Safety')}</h2>
<div class="sgroup">
${line(t('Point de restauration avant chaque installation', 'Restore point before each installation'), t('demandé, oui par défaut', 'asked, yes by default'), 'ok', HELP.restore())}
${line(t('Points de restauration gardés', 'Restore points kept'), t('5 derniers de Drivn', 'Drivn’s 5 latest'))}
${line(t('Sauvegardes de drivers gardées', 'Driver backups kept'), t('5 dernières installations', '5 latest installations'), '', HELP.backup())}
${line(t('Fichier non signé ou modifié', 'Unsigned or changed file'), t('toujours refusé', 'always refused'), 'ok', HELP.signature())}
<div class="sline"><span>${t('Protection du système de Windows', 'Windows System Protection')}<small>${t('Doit être activée sur C: pour créer des points de restauration.', 'Must be on for C: to create restore points.')}</small></span>
<button type="button" class="btng smallbtn" data-act="open-protection">${t('Ouvrir le réglage', 'Open the setting')}</button></div>
</div>

<h2>${t('Affichage', 'Display')}</h2>
<div class="sgroup">
<div class="sline"><span>${t('Animations', 'Animations')}<small>${motion === 'auto' ? t(`Auto : ${resolveMotion('auto') === 'lite' ? 'légères sur ce PC' : 'complètes sur ce PC'}`, `Auto: ${resolveMotion('auto') === 'lite' ? 'light on this PC' : 'full on this PC'}`) : t('Légères : fond fixe, sans flou, moins de calcul', 'Light: still background, no blur, less work for the PC')}</small></span>
${seg(t('Animations', 'Animations'), MOTION, motion, 'motion')}</div>
<div class="sline"><span>${t('Afficher les appareils débranchés', 'Show unplugged devices')}</span>
<button type="button" class="switch${showAbsent ? ' on' : ''}" role="switch" aria-checked="${showAbsent}" aria-label="${t('Afficher les appareils débranchés', 'Show unplugged devices')}" data-act="absent"></button></div>
<div class="sline"><span>${t('Afficher tous les appareils', 'Show all devices')}<small>${t('Aussi ceux qui ont un driver intégré à Windows (volumes, ports, sons…)', 'Also those with a driver built into Windows (volumes, ports, sounds…)')}</small></span>
<button type="button" class="switch${showAll ? ' on' : ''}" role="switch" aria-checked="${showAll}" aria-label="${t('Afficher tous les appareils', 'Show all devices')}" data-act="all"></button></div>
</div>

<h2>${t('Données', 'Data')}</h2>
<div class="sgroup">
${line(t('Télémétrie, compte, pub', 'Telemetry, account, ads'), t('aucun', 'none'), 'ok')}
${line(t('Fichiers de Drivn', 'Drivn’s files'), html`<span class="mono">%ProgramData%\\Drivn · %LocalAppData%\\Drivn</span>`)}
</div>
</section>`;
}
