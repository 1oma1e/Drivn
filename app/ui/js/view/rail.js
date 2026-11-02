// Left navigation rail.
import { html } from '../lib/dom.js';
import { t } from '../lib/i18n.js';
import { icon } from './icons.js';
import { view } from '../model/selectors.js';

const NAV = [
  ['scan', () => t('Scan', 'Scan'), 'scan'],
  ['drivers', () => t('Drivers', 'Drivers'), 'list'],
  ['apps', () => t('Applis', 'Apps'), 'sw'],
  ['games', () => t('Jeux', 'Games'), 'game'],
  ['history', () => t('Historique', 'History'), 'history'],
  ['settings', () => t('Réglages', 'Settings'), 'gear'],
];

const LOGO = html`<svg class="logo" width="36" height="36" viewBox="0 0 40 40" role="img" aria-label="Drivn">
<defs><linearGradient id="lg" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="#22D3EE"/><stop offset=".45" stop-color="#6366F1"/><stop offset="1" stop-color="#D946EF"/></linearGradient>
<radialGradient id="lgh" cx=".3" cy=".15" r=".8"><stop offset="0" stop-color="#fff" stop-opacity=".45"/><stop offset="1" stop-color="#fff" stop-opacity="0"/></radialGradient></defs>
<rect x="2" y="2" width="36" height="36" rx="11" fill="url(#lg)"/><rect x="2" y="2" width="36" height="36" rx="11" fill="url(#lgh)"/>
<path d="M13 10v20M13 10h6a10 10 0 0 1 0 20h-2.5M19.5 27l-3 3 3 3" fill="none" stroke="#fff" stroke-width="2.6" stroke-linecap="round" stroke-linejoin="round"/></svg>`;

export function railView(s) {
  const at = Math.max(0, NAV.findIndex((n) => n[0] === s.screen));
  // Each tab counts its own rows (ADR-0035).
  const badges = { drivers: view(s, 'drivers').found, apps: view(s, 'apps').found };
  return html`${LOGO}
<span class="railind" style="--i:${at}"></span>
${NAV.map(([key, label, ic], i) => html`<button type="button" class="railbtn${i === at ? ' on' : ''}" data-act="nav" data-to="${key}" aria-current="${i === at ? 'page' : 'false'}">
${icon(ic, 22, 1.7)}${label()}${badges[key] > 0 && i !== at ? html`<span class="railbadge">${badges[key]}</span>` : ''}</button>`)}
${s.update ? html`<button type="button" class="railupdate" data-act="update-ask" title="${t(`Drivn ${s.update.version} est disponible`, `Drivn ${s.update.version} is available`)}">${icon('arrowup', 18, 2.2)}${t('Mettre à jour', 'Update')}</button>` : ''}`;
}
