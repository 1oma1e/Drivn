// Drivers and Apps screens: verdict banner, running pass, tabs, search and one row per device or app.
import { html } from '../lib/dom.js';
import { t, shortDate, hhmm, longDate, shortName, elapsed, sep, s as agree } from '../lib/i18n.js';
import { icon, deviceIcon } from './icons.js';
import { view, isUpdate, isInstallableAnnounce, isOfficialTool, isJava, isPackage, isApp } from '../model/selectors.js';
import { result, summary } from './results.js';
import { sourceName, versions, fromWhere, groupLabel } from '../model/report.js';
import { tip, HELP } from './parts.js';

const TABS = [['up', () => t('À mettre à jour', 'To update'), 'arrowup'], ['ok', () => t('À jour', 'Up to date'), 'check'], ['unk', () => t('Inconnus', 'Unknown'), 'help']];
const RANK = { update: 0, update_risky: 1, announced: 2 };
const CONFETTI = html`<span class="confetti" aria-hidden="true">${Array.from({ length: 12 }, () => html`<span></span>`)}</span>`;

function hero(s, v) {
  const r = s.result;
  const apps = s.screen === 'apps';
  // ADR-0017: the age of the Windows Update list is always shown (R-VRAI-04).
  const last = t(`Scan de ${r.at} · ${s.info.total} appareils`, `Scan at ${r.at} · ${s.info.total} devices`) + (r.wu.when ? t(` · Windows Update : ${r.wu.when}`, ` · Windows Update: ${r.wu.when}`) : '');
  if (!r.wu.ok) return { tone: 'warn', ic: 'help', title: t('Pas de comparaison,', 'No comparison,'), em: t('pas de verdict.', 'no verdict.'), sub: `${r.wu.reason} ${t('Ton matériel est lu ; relance un scan pour comparer.', 'Your hardware is read; scan again to compare.')}` };
  if (s.justDone && s.last) {
    const p = s.last;
    const sub = summary(p.items, p.sim).map((c) => c.text.slice(2)).join(' · ');
    const good = p.items.some((it) => it.status === 'ok' || it.status === 'pending');
    return p.sim
      ? { tone: 'ok', ic: 'check', title: t('Simulation', 'Simulation'), em: t('terminée.', 'done.'), confetti: good, sub: `${t('Rien n’a été installé', 'Nothing was installed')} · ${sub || t('aucun élément', 'no item')}` }
      : { tone: good ? 'ok' : 'warn', ic: good ? 'check' : 'help', title: t('Installation', 'Installation'), em: p.stopped ? t('arrêtée.', 'stopped.') : t('terminée.', 'done.'), confetti: good && !p.stopped,
        sub: `${sub || t('aucun élément traité', 'no item handled')} · ${t('relance un scan pour relire les versions', 'scan again to read the versions')}` };
  }
  // R-VRAI-04: a source that failed is said, with its reason.
  const failed = (r.sources ?? []).map((e) => t(`${e.name} : pas pu vérifier (${e.why})`, `${e.name}: could not check (${e.why})`));
  // Updates from the publishers count as updates; "all up to date" only when nothing is left to check.
  if (!v.pending && v.announced) {
    return { tone: 'up', ic: 'arrowup', num: v.announced, title: '',
      em: apps ? (v.announced > 1 ? t('applis à mettre à jour', 'apps to update') : t('appli à mettre à jour', 'app to update')) : v.announced > 1 ? t('mises à jour trouvées', 'updates found') : t('mise à jour trouvée', 'update found'),
      sub: [apps ? t('Mises à jour par winget, l’outil de Windows · « Tout mettre à jour » ne prend que les applis', 'Updated by winget, the Windows tool · “Update all” takes the apps only')
        : t('« Tout mettre à jour » installe tous les drivers d’un coup, jamais les applis', '“Update all” installs every driver at once, never the apps'), ...failed, last].join(' · ') };
  }
  if (!v.pending && v.toCheck) {
    return { tone: 'risk', ic: 'help', title: t('Rien de nouveau', 'Nothing new'), em: t('trouvé.', 'found.'), sub: [t(`${v.toCheck} à vérifier chez le fabricant`, `${v.toCheck} to check at the maker`), ...failed, last].join(' · ') };
  }
  if (!v.pending) return { tone: 'ok', ic: 'check', title: t('Tout est', 'All'), em: t('à jour.', 'up to date.'), sub: [...failed, last].join(' · ') };
  return { tone: 'up', ic: 'arrowup', num: v.found, title: '', em: v.found > 1 ? t('drivers à mettre à jour', 'drivers to update') : t('driver à mettre à jour', 'driver to update'),
    sub: `${v.counts.update ? t('Faits pour ton matériel, depuis Windows Update · signature vérifiée à l’installation', 'Made for your hardware, from Windows Update · signature checked at install') : t('À vérifier avant d’installer', 'To check before installing')} · ${last}` };
}

const STAGES = () => [[t('Vérifier', 'Check'), HELP.hardwareId()], [t('Sauvegarder', 'Back up'), HELP.backup()], [t('Installer', 'Install'), null], [t('Signature', 'Signature'), HELP.signature()]];
const RING = 2 * Math.PI * 44;
const itemOf = (s, id) => s.result.items.find((x) => x.dev.id === id);

/** Progress ring: the restore point counts as one step, then one per item. */
const ring = (pct, big, small, tone = '') => html`<div class="iring ${tone}">
<svg viewBox="0 0 100 100" aria-hidden="true"><defs><linearGradient id="ig" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="#38BDF8"/><stop offset=".5" stop-color="#818CF8"/><stop offset="1" stop-color="#E879F9"/></linearGradient></defs>
<circle class="ibg" cx="50" cy="50" r="44"/><circle class="ifg" cx="50" cy="50" r="44" style="stroke-dashoffset:${(RING * (1 - pct)).toFixed(1)}"/></svg>
<span class="ival"><b>${big}</b><small>${small}</small></span></div>`;

/** One dot per item of the pass; the name and what happened are in the tooltip. */
function dot(s, id, st, why, real) {
  const x = itemOf(s, id);
  const [tone, mark] = result(st, real);
  const name = x ? shortName(x.dev.name) : id;
  return html`<span class="idot ${tone}" data-key="d${id}" title="${name}${why ? `${sep()}${why}` : ''}">${icon(x ? deviceIcon(x.dev) : 'chip', 18, 1.8)}${st === 'queued' || st === 'run' ? '' : html`<i>${mark}</i>`}</span>`;
}

const RESTORE = {
  wait: ['dim', () => t('Point de restauration', 'Restore point')], run: ['run', () => t('Point de restauration…', 'Restore point…')], ok: ['ok', () => t('Point de restauration créé', 'Restore point created')],
  warn: ['warn', () => t('Point de restauration impossible', 'Restore point failed')], skip: ['warn', () => t('Sans nouveau point de restauration', 'No new restore point')], sim: ['dim', () => t('Pas de point de restauration (simulation)', 'No restore point (simulation)')],
};

// The running pass replaces the banner: one place to look, nothing said twice.
function passPanel(s) {
  const p = s.pass;
  const ready = !['wait', 'run', 'warn'].includes(p.restore.st);
  const pct = ((ready ? 1 : 0) + p.done) / (p.ids.length + 1);
  const cur = p.current && s.rows[p.current];
  const stages = STAGES().slice(0, p.real ? 4 : 3);
  const [rtone, base] = RESTORE[p.restore.st];
  // Windows' own point of less than 24 h is said with its number: it covers the pass too.
  const rtext = p.restore.st === 'skip' && p.restore.detail ? t(`Point de restauration : ${p.restore.detail}`, `Restore point: ${p.restore.detail}`) : base();
  const title = cur ? shortName(itemOf(s, p.current)?.dev.name ?? '')
    : p.restore.st === 'run' ? t('Point de restauration', 'Restore point') : p.restore.st === 'warn' ? t('En attente de ta réponse', 'Waiting for your answer')
    : p.stopReq ? t('Arrêt en cours', 'Stopping') : p.paused ? t('En pause', 'Paused') : p.done === p.ids.length ? t('Fin de l’installation', 'Finishing') : t('Élément suivant', 'Next item');
  // Before the installer answers (UAC, start), and while Windows makes the point: what runs and since when.
  const waitingStart = p.restore.st === 'wait' && !cur;
  const sub = cur ? html`${cur.text}${cur.hint ? html`<em class="warn-t"> · ${cur.hint}</em>` : ''}`
    : p.restore.st === 'run' ? t(`Windows crée le point avant toute installation… · ${elapsed(Date.now() / 1000 - (p.restore.since ?? p.at))}`, `Windows creates the point before any install… · ${elapsed(Date.now() / 1000 - (p.restore.since ?? p.at))}`)
    : waitingStart ? t(`Windows demande l’autorisation administrateur, puis l’installeur démarre… · ${elapsed(Date.now() / 1000 - p.at)}`, `Windows asks for administrator permission, then the installer starts… · ${elapsed(Date.now() / 1000 - p.at)}`)
    : p.paused ? t('Reprends quand tu veux.', 'Resume whenever you want.') : '';
  const note = p.stopReq ? t('Arrêt après l’élément en cours', 'Stops after the current item') : p.paused && p.current ? t('Pause après cet élément', 'Pauses after this item') : '';
  return html`<div class="ipanel${p.paused ? ' paused' : ''}" data-key="ipanel">
${ring(pct, p.done, `/ ${p.ids.length}`)}
<div class="imain">
<span class="ieye">${p.real ? t('Installation', 'Installation') : html`Simulation<span class="tag">${t('rien n’est installé', 'nothing is installed')}</span>${tip(HELP.simulation())}`}</span>
<h2 class="inow" data-key="n${p.current ?? title}">${title}</h2>
<span class="isub">${sub}${note ? html` <em class="warn-t">· ${note}</em>` : ''}</span>
${cur && cur.stage >= 0 ? html`<div class="istages">${stages.map(([label, help], k) => html`<span class="istage ${k < cur.stage ? 'done' : k === cur.stage ? 'on' : ''}"><i></i>${label}${help ? tip(help) : ''}</span>`)}</div>` : ''}
</div>
<div class="ictl">
<button type="button" class="btng iconbtn${p.paused ? ' on' : ''}" data-act="pause" title="${p.paused ? t('Reprendre', 'Resume') : t('Pause après cet élément', 'Pause after this item')}" aria-label="${p.paused ? t('Reprendre', 'Resume') : t('Pause', 'Pause')}" ${p.stopReq ? 'disabled' : ''}>${icon(p.paused ? 'play' : 'pause', 18)}</button>
<button type="button" class="btng iconbtn" data-act="stop-ask" title="${t('Annuler', 'Cancel')}" aria-label="${t('Annuler', 'Cancel')}" ${p.stopReq ? 'disabled' : ''}>${icon('close', 18, 2.4)}</button>
</div>
<div class="ifoot">
<span class="idots">${p.ids.map((id) => { const r = s.rows[id] ?? { st: 'queued' }; return dot(s, id, r.st, r.st === 'run' ? r.text : r.why, r.real); })}</span>
<span class="ichip ${rtone}">${icon('shield', 14, 2)}${rtext}${tip(HELP.restore())}</span>
${p.real ? html`<span class="ihint">${t('Ne pas éteindre le PC', 'Do not turn off the PC')}</span>` : ''}
<button type="button" class="link" data-act="journal" data-from="live">${t('Journal', 'Log')}</button>
</div></div>`;
}

// R-SEC-06: shown at start-up when a journal has no end; nothing happens without a choice.
function cutPanel(s) {
  const c = s.interrupted;
  const cut = c.started ? 1 : 0;
  const n = c.items.length + cut + c.rest.length;
  return html`<div class="ipanel cut" data-key="cut">
${ring(c.items.length / n, c.items.length, `/ ${n}`, 'warn')}
<div class="imain">
<span class="ieye">${t('Installation interrompue', 'Interrupted installation')}</span>
<h2 class="inow">${t('Le PC s’est éteint pendant une installation', 'The PC turned off during an installation')}</h2>
<span class="isub">${t(`Le ${longDate(c.at)} à ${hhmm(c.at)}. Rien n’est fait sans ton choix`, `On ${longDate(c.at)} at ${hhmm(c.at)}. Nothing is done without your choice`)}${c.restore ? t(' ; le point de restauration de cette installation est toujours là', '; this installation’s restore point is still there') : ''}.</span>
</div>
<div class="ifoot">
<span class="idots">${c.items.map((it) => dot(s, it.id, it.status ?? 'ok', it.why, !c.sim))}${cut ? dot(s, c.started, 'unknown', t('commencé, pas fini : sera relu en premier', 'started, not finished: read again first'), !c.sim) : ''}${c.rest.map((id) => dot(s, id, 'queued', t('pas commencé', 'not started'), !c.sim))}</span>
<span class="ichip">${t(`${c.items.length} fini${agree(c.items.length)}`, `${c.items.length} done`)}${cut ? t(' · 1 à relire', ' · 1 to read again') : ''} · ${t(`${c.rest.length} pas commencé${agree(c.rest.length)}`, `${c.rest.length} not started`)}</span>
<button type="button" class="link" data-act="journal" data-from="cut">${t('Journal', 'Log')}</button>
</div>
<div class="icut">
<button type="button" class="btnp smallbtn" data-act="cut-resume">${icon('play', 14)}${t('Reprendre', 'Resume')}</button>
<button type="button" class="btng smallbtn" data-act="cut-keep">${t('Arrêter et garder', 'Stop and keep')}</button>
<button type="button" class="btng smallbtn" data-act="cut-revert">${icon('undo', 14)}${t('Tout remettre comme avant', 'Put everything back')}</button>
</div></div>`;
}

/** Second line of a row: dates, or why there is nothing newer (R-VRAI-04). */
function dates(d, dec, o) {
  if (d.tool === 'winget') return t('mise à jour par winget, l’outil de Windows', 'updated by winget, the Windows tool');
  if (d.tool === 'java') return t('version lue chez Oracle · installée par Java Update', 'version read at Oracle · installed by Java Update');
  // A date the source does not give is left out, never shown as a dash.
  if (o) return [d.inst?.d && shortDate(d.inst.d), o.d && shortDate(o.d)].filter(Boolean).join(' → ');
  const old = d.inst;
  if (d.bios && dec.state === 'up_to_date') return `${old?.d ? `${shortDate(old.d)} · ` : ''}${t(`le dernier chez ${old.p}`, `the latest at ${old.p}`)}`;
  if (dec.state === 'up_to_date') {
    const more = dec.hints.some((h) => h.kind === 'publisher_lists_newer') ? t(' · le fabricant de la carte mère publie plus récent', ' · the motherboard maker publishes newer')
      : dec.hints.length ? t(' · le fabricant a peut-être plus récent', ' · the maker may have newer')
      : dec.unchecked.length ? t(` · pas pu vérifier chez ${dec.unchecked.join(', ')}`, ` · could not check at ${dec.unchecked.join(', ')}`) : '';
    return `${old ? `${shortDate(old.d)} · ` : ''}${t('rien de plus récent sur Windows Update', 'nothing newer on Windows Update')}${more}`;
  }
  if (dec.state === 'no_driver') return t('rien sur Windows Update', 'nothing on Windows Update');
  return dec.reason.replace(/\.$/, '');
}

function row(s, v, x, i) {
  const { dev: d, dec } = x;
  const r = s.rows[d.id] ?? {};
  const busy = !!s.pass || !!s.interrupted;
  const done = r.st === 'ok';
  const isUp = dec.state === 'update';
  const isRisk = dec.state === 'update_risky';
  // R-VER-03: a version announced by another source is never "to update"; never ticked by default (R-SRC-06).
  const isAnn = dec.state === 'announced';
  const o = dec.offer;
  const doubt = dec.unchecked.length || dec.hints.length;
  const tone = done ? 'ok' : isUp ? 'up' : isRisk || isAnn ? 'risk' : dec.state === 'up_to_date' && !doubt ? 'ok' : '';
  const shown = o ? versions(d.inst, o) : null;
  const cur = shown ? shown.cur : d.inst ? d.inst.v : t('aucun', 'none');
  const name = shortName(d.name);
  const picked = v.isPicked(x);
  const off = busy ? 'disabled' : '';
  const update = t('Mettre à jour', 'Update');

  let action = '';
  if (!r.st) {
    action = isUp || isApp(x) ? html`<button type="button" class="${isUp ? 'btnp' : 'btnw'} rowbtn" data-act="install-one" data-id="${d.id}" ${off}>${update}</button>`
      : isPackage(x) ? html`<button type="button" class="btnw rowbtn" data-act="package-ask" data-id="${d.id}" ${off}>${update}</button>`
      : isJava(x) ? html`<button type="button" class="btnw rowbtn" data-act="tool-ask" data-tool="java" ${off}>${update}</button>`
      : html`<button type="button" class="${isRisk ? 'btnw' : 'btng'} rowbtn" data-act="open" data-id="${d.id}">${isRisk ? t('Voir pourquoi', 'See why') : t('Détails', 'Details')}</button>`;
  } else if (r.st === 'run' || r.st === 'queued') {
    action = html`<span class="prog"><span>${r.st === 'queued' ? t('En attente', 'Waiting') : r.text}</span><span class="track${r.busy ? ' busy' : ''}"><span class="fill" style="width:${r.prog ?? 0}%"></span></span></span>`;
  } else {
    const [rt, , label] = result(r.st, r.real);
    action = r.st === 'ok' ? html`<span class="okpill">${icon('check', 15, 2.6)}${label}</span>` : html`<span class="failpill ${rt}" title="${r.why ?? ''}">${label}</span>`;
  }

  const tags = [groupLabel(d.group), d.inst?.p,
    // No "?" here: the whole row is a button that opens the sheet, where each word is explained.
    isRisk && !done ? t('à vérifier', 'to check') : null,
    isAnn && o && !d.tool ? t(`annoncée par ${sourceName(o.src)}`, `announced by ${sourceName(o.src)}`) : null,
    o && !done ? fromWhere(x) : null, d.absent ? t('débranché', 'unplugged') : null].filter(Boolean);

  return html`<div class="row ${tone}${done ? ' done' : ''}" data-key="${d.id}" style="animation-delay:${Math.min(i * 55, 700)}ms">
${(isUp || isRisk || isInstallableAnnounce(x) || isOfficialTool(x) || isApp(x)) && !r.st ? html`<button type="button" class="tick${picked ? ' on' : ''}${busy ? ' off' : ''}" data-act="pick" data-id="${d.id}" role="checkbox" aria-checked="${picked}" aria-label="${t(`Installer ${name}`, `Install ${name}`)}">${icon('check', 14, 3.2)}</button>` : ''}
<button type="button" class="rowmain" data-act="open" data-id="${d.id}" title="${t('Voir pourquoi', 'See why')}">
<span class="ico ${tone}">${icon(deviceIcon(d), 21, 1.8)}</span>
<span class="names"><b>${name}</b><small>${tags.map((tag, k) => html`${k ? ' · ' : ''}${tag}`)}</small>${d.role ? html`<small class="role">${d.role}</small>` : ''}</span>
</button>
<span class="vbox"><span class="vline">${o ? html`<span class="mono vold">${cur}</span><span class="varrow">${icon('arrow', 14, 2.4)}</span><span class="mono vnew">${shown.nw}</span>` : html`<span class="mono vsame">${cur}</span>`}</span>
<span class="vdates">${dates(d, dec, o)}</span></span>
${action}
<span class="rowbar${r.busy ? ' busy' : ''}" style="width:${r.st === 'run' ? r.prog ?? 0 : 0}%"></span>
</div>`;
}

export function driversView(s) {
  const v = view(s);
  const apps = s.screen === 'apps';
  const h = s.pass || s.interrupted ? null : hero(s, v);
  const busy = !!s.pass || !!s.interrupted;
  const q = s.query.trim().toLowerCase();
  const inTab = (x) => {
    const st = x.dec.state;
    const r = s.rows[x.dev.id];
    if (s.tab === 'up') return isUpdate(x) || st === 'announced' || !!r?.st;
    if (s.tab === 'ok') return st === 'up_to_date' && r?.st !== 'ok';
    return st === 'unknown' || st === 'no_driver';
  };
  const match = (x) => !q || `${x.dev.name} ${groupLabel(x.dev.group)} ${x.dev.inst?.p ?? ''}`.toLowerCase().includes(q);
  // A row touched by a pass keeps its place, so nothing jumps while items install.
  const rank = (x) => (s.rows[x.dev.id] ? 0 : RANK[x.dec.state] ?? 2);
  const list = v.items.filter((x) => inTab(x) && match(x)).sort((a, b) => rank(a) - rank(b));
  const counts = { up: v.updates + v.announced, ok: v.counts.up_to_date, unk: v.unknown };
  const tabs = TABS.filter(([k]) => k !== 'unk' || counts.unk);
  const all = v.pickable.length > 0 && v.pickable.every(v.isPicked);
  const empty = q ? t('Rien ne correspond.', 'Nothing matches.') : s.tab === 'up' ? t('Rien à mettre à jour. Tout va bien.', 'Nothing to update. All good.') : s.tab === 'ok' ? t('Rien ici pour l’instant.', 'Nothing here yet.') : t('Aucun inconnu.', 'No unknown.');

  return html`<section class="drivers" data-key="drivers">
<header class="dhead">
${s.pass ? passPanel(s) : s.interrupted ? cutPanel(s) : html`<div class="hero ${h.tone}">
${h.confetti ? CONFETTI : ''}
<div class="rise herotext">
<span class="hbadge ${h.tone}">${icon(h.ic, 30, 2.3)}</span>
<div><h1>${h.num ? html`<span class="bignum">${h.num}</span>` : ''}<span>${h.title} <em class="serif shine">${h.em}</em></span></h1>
<span class="sub">${h.sub}</span></div>
</div>
<div class="heroact">
<button type="button" class="btng iconbtn" data-act="rescan" aria-label="${t('Scanner à nouveau', 'Scan again')}" title="${t('Scanner à nouveau', 'Scan again')}" ${busy ? 'disabled' : ''}>${icon('rescan', 20)}</button>
${v.pickable.length && !busy ? html`<button type="button" class="btnp bigbtn" data-act="update-all">${icon('arrowup', 18, 2.4)}<span>${t(`Tout mettre à jour (${v.pickable.length})`, `Update all (${v.pickable.length})`)}</span></button>` : ''}
${v.pickable.length || busy ? html`<button type="button" class="btng smallbtn" data-act="install-sel" ${busy || !v.selected.length ? 'disabled' : ''}>${v.selected.length ? t(`Seulement la sélection (${v.selected.length})`, `Selection only (${v.selected.length})`) : t('Rien de coché', 'Nothing ticked')}</button>` : ''}
</div></div>`}
${s.reboot.length && !busy ? html`<div class="reboot fade"><span>${t(`Redémarrage en attente : ${s.reboot.map(shortName).join(', ')}. Drivn ne redémarre jamais tout seul.`, `Restart pending: ${s.reboot.map(shortName).join(', ')}. Drivn never restarts on its own.`)}</span>
<span class="btns"><button type="button" class="btnp smallbtn" data-act="reboot-now">${t('Redémarrer maintenant', 'Restart now')}</button></span></div>` : ''}
<div class="stats" role="tablist">${tabs.map(([k, label, ic], i) => html`<button type="button" role="tab" aria-selected="${s.tab === k}" class="stat${s.tab === k ? ' on' : ''}" style="animation-delay:${120 + i * 80}ms" data-act="tab" data-to="${k}">
<span class="sico ${k}">${icon(ic, 20)}</span><span class="stext"><span class="snum">${counts[k]}</span><span class="slab">${label()}</span></span></button>`)}</div>
</header>
<div class="dlist">
<div class="toolbar">
<label class="search">${icon('search', 16)}<input type="search" placeholder="${apps ? t('Chercher une appli', 'Search an app') : t('Chercher un appareil', 'Search a device')}" aria-label="${t('Chercher', 'Search')}" value="${s.query}" data-input="query"></label>
${s.tab === 'up' && v.pickable.length && !busy ? html`<span class="seltext">${t(`${v.selected.length} sur ${v.pickable.length} cochée${agree(v.selected.length)}`, `${v.selected.length} of ${v.pickable.length} ticked`)}</span>
<button type="button" class="link" data-act="pick-all">${all ? t('Tout décocher', 'Untick all') : t('Tout cocher', 'Tick all')}</button>` : ''}
</div>
${list.length ? '' : html`<div class="empty fade"><span class="emptyico">${icon(q ? 'search' : 'check', 26, 2.2)}</span>${empty}</div>`}
<div class="list">${list.map((x, i) => row(s, v, x, i))}</div>
</div>
</section>`;
}
