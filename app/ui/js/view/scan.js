// Scan screen: the neon orb button, its progress ring and the device families read so far.
import { html } from '../lib/dom.js';
import { t, plural, shortName, elapsed } from '../lib/i18n.js';
import { icon, deviceIcon } from './icons.js';
import { view, isUpdate } from '../model/selectors.js';
import { groupLabel, sourceName } from '../model/report.js';

const RING = 1005.31; // 2 * PI * 160, the progress circle
const PROMISES = [['shield', () => t('Sources officielles', 'Official sources')], ['check', () => t('Compatible avec ton matériel', 'Compatible with your hardware')], ['undo', () => t('Retour arrière en un clic', 'One-click rollback')]];

const MARK = html`<svg class="cmark" viewBox="0 0 40 40" aria-hidden="true"><path class="cm-stem" d="M13 10v20"/><path class="cm-arc" d="M13 10h6a10 10 0 0 1 0 20h-2.5" pathLength="60"/><path class="cm-head" d="M19.5 27l-3 3 3 3"/></svg>`;

/** One chip per device family, ticked as it is read; "↑ n" once Windows Update answered. */
function families(devices, p1, peek) {
  const read = Math.floor((p1 / 100) * devices.length + 1e-4);
  const groups = new Map();
  devices.forEach((d, i) => {
    let g = groups.get(d.group);
    if (!g) groups.set(d.group, (g = { key: d.group, icon: deviceIcon(d), total: 0, read: 0, up: 0 }));
    g.total++;
    if (i < read) g.read++;
  });
  for (const x of peek ?? []) if (isUpdate(x)) groups.get(x.dev.group).up++;
  const current = read < devices.length ? devices[read].group : null;
  return {
    now: devices.length ? shortName(devices[Math.min(devices.length - 1, read)].name) : '',
    chips: [...groups.values()].map((g, i) => {
      const st = g.read === g.total ? (g.up ? 'up' : 'ok') : g.key === current ? 'run' : 'wait';
      return html`<span class="chip ${st}" style="animation-delay:${i * 50}ms">${icon(g.icon, 15)}${groupLabel(g.key)}<span class="cst ${st}">${st === 'ok' ? '✓' : st === 'up' ? `↑ ${g.up}` : ''}</span></span>`;
    }),
  };
}

/** What the scan does now, in words, with the detail under it (Spec - Scan complet). */
function step(s) {
  const { phase, vendors, apps, since } = s.scan;
  const spent = since ? ` · ${elapsed(Date.now() / 1000 - since)}` : '';
  if (phase === 'read') {
    const now = s.scan.current ? shortName(s.scan.current) : '';
    return [t('Je lis ton matériel', 'Reading your hardware'), now ? t(`Lecture : ${now}`, `Reading: ${now}`) : t('appareils et drivers installés…', 'devices and installed drivers…'), `r${now}`];
  }
  if (apps) return [t('Je demande à winget', 'Asking winget'), t(`applications installées et leurs mises à jour…${spent}`, `installed apps and their updates…${spent}`), 'apps'];
  if (vendors?.source && vendors.about) {
    return [t(`Je demande à ${sourceName(vendors.source)}`, `Asking ${sourceName(vendors.source)}`),
      t(`${shortName(vendors.about)} · ${Math.min(vendors.done + 1, vendors.total)} sur ${vendors.total}`, `${shortName(vendors.about)} · ${Math.min(vendors.done + 1, vendors.total)} of ${vendors.total}`), `v${vendors.done}`];
  }
  if (vendors) return [t('Je demande aux fabricants', 'Asking the makers'), t(`${vendors.done} sur ${vendors.total} vérifiés`, `${vendors.done} of ${vendors.total} checked`), 'v'];
  // Windows Update gives no progress (ADR-0018): the time spent shows it still works.
  return [t('Je cherche sur Windows Update', 'Searching Windows Update'), t(`Windows Update ne donne pas d’avancement : ça peut prendre une minute${spent}`, `Windows Update gives no progress: it can take a minute${spent}`), 'w'];
}

export function scanView(s) {
  const { phase, p1, peek } = s.scan;
  const scanning = phase === 'read' || phase === 'search';
  const found = phase === 'found';
  const launching = phase === 'launch';
  const wu = s.result?.wu;
  const v = view(s);
  const nDev = s.info.devices.length;
  // Percent of the devices read (R-VRAI-01). The online searches that follow give no progress:
  // the ring then turns without a number (ADR-0018).
  const waiting = phase === 'search' && !s.scan.vendors;
  const pct = found ? 100 : Math.floor(s.scan.pct ?? 0);

  let title, em, core, below;
  if (scanning) {
    const reading = phase === 'read';
    const fam = families(s.info.devices, reading ? p1 ?? 0 : 100, peek);
    title = t('Je fais le tour', 'Going through');
    em = reading ? t('de ton PC…', 'your PC…') : t('des mises à jour…', 'the updates…');
    // After Windows Update, the publishers' sites, one question at a time (R-SRC-11).
    // Their progress is measured: questions asked out of questions planned (R-VRAI-01).
    const vendors = !reading && !s.scan.apps && s.scan.vendors;
    const vpct = vendors && vendors.total ? Math.floor((vendors.done * 100) / vendors.total) : 0;
    core = vendors
      ? html`<span class="cin" data-key="vendors" role="progressbar" aria-valuemin="0" aria-valuemax="100" aria-valuenow="${vpct}"><span class="cpct">${vpct}<small>%</small></span><span class="cdiv"></span><span class="csub">${t('Fabricants', 'Makers')}</span></span>`
      : !reading
      ? html`<span class="cin" data-key="${s.scan.apps ? 'apps' : 'online'}"><span class="cok">${icon(s.scan.apps ? 'sw' : 'wifi', 36, 2.3)}</span><span class="cdiv"></span><span class="csub">${s.scan.apps ? 'winget' : 'Windows Update'}</span></span>`
      : html`<span class="cin" data-key="scan" role="progressbar" aria-valuemin="0" aria-valuemax="100" aria-valuenow="${pct}"><span class="cpct">${pct}<small>%</small></span><span class="cdiv"></span><span class="csub">${t('Scan en cours', 'Scanning')}</span></span>`;
    const [what, detail, key] = step(s);
    below = html`<span class="fade now"><strong>${what}</strong>
<span class="mono" data-key="${key}">${detail}</span></span>
<div class="chips">${fam.chips}</div>
<button type="button" class="pill" data-act="scan-stop">${t('Arrêter', 'Stop')}</button>`;
  } else if (found) {
    const sub = t(`sur ${nDev} composants · ${s.info.total} appareils lus`, `out of ${nDev} components · ${s.info.total} devices read`);
    title = t('C’est fini,', 'All done,');
    em = t('voici le bilan.', 'here is the result.');
    core = !wu.ok
      ? html`<span class="cin" data-key="found"><span class="cok warn">${icon('help', 36, 2.5)}</span><span class="csub">${t('Pas de verdict', 'No verdict')}</span></span>`
      : v.found
        ? html`<span class="cin" data-key="found"><span class="cbig">${v.found}</span><span class="csub">${v.found > 1 ? t('mises à jour trouvées', 'updates found') : t('mise à jour trouvée', 'update found')}</span></span>`
        // R-VRAI-04: never "all up to date" while a publisher could not be checked.
        : v.toCheck
          ? html`<span class="cin" data-key="found"><span class="cok warn">${icon('help', 36, 2.5)}</span><span class="csub">${t('Rien de nouveau trouvé', 'Nothing new found')}</span></span>`
          : html`<span class="cin" data-key="found"><span class="cok">${icon('check', 36, 2.5)}</span><span class="csub">${t('Tout est à jour', 'All up to date')}</span></span>`;
    below = html`<span class="fade sub">${wu.ok ? sub : wu.reason}</span>
<button type="button" class="wbtn rise2" data-act="results"><span>${t('Voir les résultats', 'See the results')}</span><span class="warr">${icon('arrow', 16, 2.4)}</span></button>`;
  } else {
    title = t('Plus récent, c’est bien.', 'Newer is good.');
    em = t('Compatible, c’est mieux.', 'Compatible is better.');
    core = html`<span class="cin" data-key="idle">${MARK}<span class="clab">${t('Scanner', 'Scan')}</span><span class="cdiv"></span>
<span class="csub">${s.result ? t(`dernier · ${s.result.at}`, `last · ${s.result.at}`) : t('mes drivers', 'my drivers')}</span></span>`;
    const idleSub = !s.result ? t('Environ une minute · lecture seule · rien n’est modifié', 'About a minute · read-only · nothing is changed')
      : !wu.ok ? t('Rien n’a été comparé au dernier scan', 'Nothing was compared at the last scan')
      : v.pending ? t(`${plural(v.pending, ['mise à jour', 'mises à jour'], ['update', 'updates'])} en attente`, `${plural(v.pending, ['mise à jour', 'mises à jour'], ['update', 'updates'])} waiting`)
      : v.announced || v.toCheck ? t('Rien de nouveau sur Windows Update · à vérifier chez les fabricants', 'Nothing new on Windows Update · to check at the makers') : t('Tout est à jour', 'All up to date');
    below = html`<span class="rise3 sub">${idleSub}</span>
<span class="rise3 promises">${PROMISES.map(([ic, text]) => html`<span class="promise">${icon(ic, 15)}${text()}</span>`)}</span>
${s.result ? html`<button type="button" class="pill" data-act="results">${t('Voir les résultats →', 'See the results →')}</button>` : ''}`;
  }

  const stage = scanning ? `scan${waiting || s.scan.apps ? ' indet' : ''}` : found ? 'found' : launching ? 'go' : 'idle';
  return html`<section class="scan" data-key="scan">
<span class="eyebrow rise"><i class="live"></i>${s.result ? t(`Dernière analyse à ${s.result.at}`, `Last scan at ${s.result.at}`) : t('Ton PC', 'Your PC')}</span>
<h1 class="rise2">${title} <em class="serif shine">${em}</em></h1>
<div class="stage ${stage}">
<span class="fx" aria-hidden="true"><span class="bloom"></span><span class="orbit o2"><i></i></span><span class="orbit o1"><i></i></span>
<span class="sonar"></span><span class="sonar s2"></span><span class="sonar s3"></span><span class="hring"></span>
<svg class="pring" viewBox="0 0 360 360"><defs><linearGradient id="lgP" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="#E0F2FE"/><stop offset=".5" stop-color="#fff"/><stop offset="1" stop-color="#F5D0FE"/></linearGradient></defs>
<circle cx="180" cy="180" r="160" fill="none" stroke="rgba(255,255,255,.1)" stroke-width="3"/>
<circle class="pr" cx="180" cy="180" r="160" fill="none" stroke="url(#lgP)" stroke-width="4" stroke-linecap="round" stroke-dasharray="${RING}" style="stroke-dashoffset:${(RING * (1 - pct / 100)).toFixed(1)}" transform="rotate(-90 180 180)"/></svg>
<span class="shock"></span></span>
<button type="button" class="core" data-act="core" aria-label="${scanning ? t('Scan en cours', 'Scanning') : found ? t('Voir les résultats', 'See the results') : s.result ? t('Rescanner mes drivers', 'Scan my drivers again') : t('Scanner mes drivers', 'Scan my drivers')}">${core}</button>
</div>
<div class="below">${below}</div>
</section>`;
}
