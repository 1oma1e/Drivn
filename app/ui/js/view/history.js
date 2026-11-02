// History screen: one card per pass, rollback driver by driver (R-SEC-03), 5 passes kept (R-SEC-07).
import { html } from '../lib/dom.js';
import { t, hhmm, longDate, plural, shortName, s as agree } from '../lib/i18n.js';
import { icon, deviceIcon } from './icons.js';
import { result } from './results.js';
import { tip, HELP } from './parts.js';

function passCard(s, p, i) {
  const open = s.open[p.id] ?? i === 0;
  const ok = p.items.filter((it) => it.status === 'ok').length;
  const when = `${longDate(p.at)} ${hhmm(p.at)}`;
  const meta = [plural(p.items.length, ['élément', 'éléments'], ['item', 'items']),
    p.sim ? t('simulation, rien d’installé', 'simulation, nothing installed') : p.restore ? t('point de restauration ✓', 'restore point ✓') : t('sans point de restauration', 'no restore point')];
  if (p.stopped) meta.push(p.stopped === 'revert' ? t('annulée, tout remis', 'cancelled, all put back') : t(`arrêtée, ${p.left} pas installé${agree(p.left)}`, `stopped, ${p.left} not installed`));
  return html`<div class="pcard${p.expired ? ' expired' : ''}" data-key="p${p.id}">
<button type="button" class="phead" data-act="toggle-pass" data-id="${p.id}" aria-expanded="${open}">
<span class="chev${open ? ' open' : ''}">${icon('chevron', 16)}</span><b>${when}</b><span class="pmeta">${meta.join(' · ')}</span>
${p.expired ? html`<span class="tag warn">${t('retour impossible', 'no rollback')}</span>` : ok ? html`<span class="tag">${t(`${ok} à annuler`, `${ok} to undo`)}</span>` : ''}${p.sim ? html`<span class="tag">simulation</span>` : ''}
</button>
${open ? html`<div class="pitems">
${p.expired ? html`<span class="pnote">${t('Sauvegarde supprimée : Drivn garde les 5 dernières installations.', 'Backup deleted: Drivn keeps the 5 latest installations.')}</span>` : ''}
${p.items.map((it, j) => html`<div class="pitem ${it.status}">
<span class="ico ${it.status === 'ok' ? 'ok' : it.status === 'pending' ? '' : 'risk'}">${icon(deviceIcon(it), 18, 1.8)}</span>
<span class="pname">${shortName(it.name)}</span>
<span class="mono pver">${it.status === 'rolled' ? `${it.to} → ${it.from}` : it.status === 'ok' || it.status === 'pending' ? `${it.from} → ${it.to}` : it.from}</span>
${it.status === 'ok' && it.n != null && !p.expired ? html`<button type="button" class="btng smallbtn" data-act="rollback-ask" data-pass="${p.id}" data-index="${j}" ${s.pass ? 'disabled' : ''}>${icon('undo', 14)}${t('Revenir', 'Revert')}</button>`
  : html`<span class="pstat" title="${it.why ?? ''}">${it.status === 'pending' ? t('redémarre d’abord', 'restart first') : result(it.status, !p.sim)[2]}</span>`}
</div>`)}
</div>` : ''}
</div>`;
}

export function historyView(s) {
  const list = s.history;
  const point = list.find((p) => p.restore && !p.expired);
  return html`<section class="page rise" data-key="history">
<h1>${t('Historique', 'History')}</h1>
<span class="lead">${t('Chaque driver remplacé est sauvegardé. Un clic pour revenir.', 'Every replaced driver is backed up. One click to go back.')}${tip(HELP.backup())}</span>
${list.length ? html`<div class="plist">${list.map((p, i) => passCard(s, p, i))}</div>`
  : html`<div class="empty"><span class="emptyico">${icon('history', 26, 2)}</span>${t('Rien d’installé pour l’instant.', 'Nothing installed yet.')}</div>`}
<div class="pfoot"><span>${t('Drivn garde les sauvegardes de ses 5 dernières installations et ses 5 derniers points de restauration ; le plus ancien est supprimé.', 'Drivn keeps the backups of its 5 latest installations and its 5 latest restore points; the oldest is deleted.')}${tip(HELP.restore())}</span>
${point ? html`<button type="button" class="btng smallbtn" data-act="system-restore">${t(`Restaurer tout le PC au point du ${longDate(point.at)}`, `Restore the whole PC to the point of ${longDate(point.at)}`)}</button>` : ''}</div>
</section>`;
}
