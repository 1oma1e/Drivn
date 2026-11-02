// Dialogs (recap, restore point, stop, end of pass, journal, rollback, official tools, update) and the toast.
import { html } from '../lib/dom.js';
import { t, plural, shortName, size, sep, s as agree } from '../lib/i18n.js';
import { icon, deviceIcon } from './icons.js';
import { sheetView } from './sheet.js';
import { sourceName } from '../model/report.js';
import { result, summary, chips } from './results.js';
import { modal, buttons, dhead, tip, busy, bar, HELP } from './parts.js';

const ditem = (dev, i, sub, right) => html`<div class="ditem" style="animation-delay:${120 + i * 70}ms">
<span class="ico">${icon(deviceIcon(dev), 18, 1.8)}</span><span class="dname">${shortName(dev.name)}${sub ? html`<small>${sub}</small>` : ''}</span>${right}</div>`;
const updates = (n) => plural(n, ['mise à jour', 'mises à jour'], ['update', 'updates']);

function recap(s, d) {
  const { ids, real } = d;
  const items = ids.map((id) => s.result.items.find((x) => x.dev.id === id)).filter(Boolean);
  const n = items.length;
  const net = items.some(({ dev }) => dev.group === 'net' || dev.group === 'bt');
  const safe = real
    ? [['shield', t('Point de restauration', 'Restore point'), HELP.restore()], ['undo', t('Sauvegarde de chaque driver', 'Backup of each driver'), HELP.backup()], ['check', t('Signature vérifiée', 'Signature checked'), HELP.signature()]]
    : [['shield', t('Simulation : rien ne sera installé', 'Simulation: nothing will be installed'), HELP.simulation()]];
  const after = (d.after ?? []).map((id) => s.result.items.find((it) => it.dev.id === id))
    .map((x) => (x?.dev.tool === 'java' ? 'Java Update' : t(`l’installeur ${sourceName(x?.dec.offer?.src)}`, `${sourceName(x?.dec.offer?.src)}’s installer`)));
  return modal(t('Installer', 'Install'), html`${dhead('up', 'arrowup', real ? t(`Installer ${updates(n)}`, `Install ${updates(n)}`) : t(`Simuler ${updates(n)}`, `Simulate ${updates(n)}`),
    html`${t('Windows va demander l’autorisation administrateur.', 'Windows will ask for administrator permission.')}${tip(HELP.uac())}`)}
<div class="dsafe">${safe.map(([ic, text, help], i) => html`<span style="animation-delay:${60 + i * 60}ms">${icon(ic, 15, 2.2)}${text}${tip(help)}</span>`)}</div>
<div class="ilist">${items.map((x, i) => ditem(x.dev, i, x.dec.risk ? t('À vérifier', 'To check') : '',
    html`<span class="mono dver">${x.dev.inst?.v ?? t('aucun', 'none')}<i>${icon('arrow', 12, 2.4)}</i><b>${x.dec.offer.v}</b></span>`))}</div>
${real && net ? html`<span class="dnote">${t('Le réseau peut couper quelques secondes.', 'The network may drop for a few seconds.')}</span>` : ''}
${after.length ? html`<span class="dnote">${t(`Ensuite s’ouvrira ${after.join(', puis ')}.`, `Then ${after.join(', then ')} will open.`)}</span>` : ''}
${buttons(['dialog-close', t('Annuler', 'Cancel')], ['recap-go', real ? t('Installer', 'Install') : t('Lancer la simulation', 'Start the simulation'), true])}`, 'dialog-close');
}

function stop(s) {
  const n = s.pass.installedCount;
  return modal(t('Arrêter l’installation', 'Stop the installation'), html`${dhead('warn', 'pause', t('Arrêter l’installation ?', 'Stop the installation?'),
    `${t('En pause pendant que tu choisis. L’élément en cours va au bout.', 'Paused while you choose. The current item finishes.')} ${n ? t(`${plural(n, ['élément', 'éléments'], ['item', 'items'])} déjà installé${agree(n)}.`, `${plural(n, ['élément', 'éléments'], ['item', 'items'])} already installed.`) : t('Rien n’est encore installé.', 'Nothing is installed yet.')}`)}
<div class="dcol">${n
    ? html`<button type="button" class="btnp" data-act="stop-revert">${icon('undo', 16)}${t('Tout remettre comme avant', 'Put everything back')}</button><button type="button" class="btng" data-act="stop-keep">${t('Garder ce qui est installé', 'Keep what is installed')}</button>`
    : html`<button type="button" class="btnp" data-act="stop-keep">${t('Arrêter', 'Stop')}</button>`}
<button type="button" class="link" data-act="stop-continue">${t('Continuer l’installation', 'Continue the installation')}</button></div>`, 'stop-continue');
}

function rollback(s, { passId, index }) {
  const it = s.history.find((p) => p.id === passId)?.items[index];
  if (!it) return '';
  return modal(t('Revenir en arrière', 'Roll back'), html`<strong class="dtitle">${t('Revenir à l’ancien driver ?', 'Go back to the old driver?')}</strong>
<span class="mono dtext">${shortName(it.name)}${sep()}${it.to} → ${it.from}</span>
<span class="dnote">${t('L’appareil peut couper quelques secondes. Windows va demander l’autorisation administrateur.', 'The device may stop for a few seconds. Windows will ask for administrator permission.')}</span>
${buttons(['dialog-close', t('Annuler', 'Cancel')], ['rollback-go', t('Revenir', 'Go back'), true])}`, 'dialog-close');
}

/** Something runs that the user waits for (a rollback: UAC, then the installer). No way to close it. */
const working = (s, d) => modal(t('En cours', 'Working'), html`${dhead('up', 'undo', t('En cours…', 'Working…'), '')}
<div class="dprog">${busy(d.text, d.at)}</div>`);

// R-SEC-01: the pass waits before any install; only an explicit choice lets it go on.
const restore = (s, { reason, existing }) => modal(t('Pas de point de restauration', 'No restore point'), html`${dhead('warn', 'shield', t('Pas de point de restauration', 'No restore point'), reason)}
<span class="dtext">${t('Rien n’est encore installé. Si tu continues, chaque driver est quand même sauvegardé et peut être remis comme avant', 'Nothing is installed yet. If you go on, each driver is still backed up and can be put back')}${existing ? t(`, et le point n° ${existing}, d’avant cette installation, peut tout remettre`, `, and point #${existing}, from before this installation, can put everything back`) : ''}.</span>
<button type="button" class="link dlink" data-act="open-protection">${t('Ouvrir Protection du système', 'Open System Protection')}</button>
<div class="dbtns"><button type="button" class="btng" data-act="restore-no">${t('Annuler', 'Cancel')}</button>
<button type="button" class="btnw" data-act="restore-yes">${existing ? t(`Continuer avec le point n° ${existing}`, `Continue with point #${existing}`) : t('Continuer sans point', 'Continue without a point')}</button></div>`);

// Big badge of the end of pass: a ring and a mark drawn in.
const MARK = { ok: 'M7 12.5l3.5 3.5L17 9', wait: 'M12 7v5l3 2', warn: 'M12 7.5v5.5M12 16.5h.01', sim: 'M7 12.5l3.5 3.5L17 9' };
const endBadge = (tone) => html`<span class="endbadge ${tone}"><svg viewBox="0 0 64 64" aria-hidden="true"><circle cx="32" cy="32" r="28"/></svg>
<svg class="emark" viewBox="0 0 24 24" aria-hidden="true"><path d="${MARK[tone]}"/></svg><span class="eburst"></span></span>`;

function end(s) {
  const p = s.last;
  if (!p) return '';
  const history = s.history.find((h) => h.id === p.id);
  const reboot = !p.sim && p.reboot.length > 0; // R-RBT-01
  const good = p.items.filter((it) => it.status === 'ok').length;
  const allOk = p.items.length > 0 && good === p.items.length;
  const [tone, title, sub] = p.sim ? ['sim', t('Simulation terminée', 'Simulation done'), t('Rien n’a été installé : seulement les lectures et les vérifications.', 'Nothing was installed: only the reads and the checks.')]
    : p.stopped ? ['warn', t('Installation arrêtée', 'Installation stopped'), t('Arrêtée à ta demande, entre deux éléments.', 'Stopped at your request, between two items.')]
    : reboot ? ['wait', t('Presque fini', 'Almost done'), t('Un redémarrage termine l’installation.', 'A restart finishes the installation.')]
    : allOk ? ['ok', t('C’est à jour', 'All up to date'), t(`${plural(good, ['élément installé', 'éléments installés'], ['item installed', 'items installed'])}, déjà actif${agree(good)}.`, `${plural(good, ['élément installé', 'éléments installés'], ['item installed', 'items installed'])}, already active.`)]
    : ['warn', t('Installation terminée', 'Installation done'), t('Certaines mises à jour n’ont pas été faites : le détail est ci-dessous.', 'Some updates were not done: the details are below.')];
  const notes = [];
  // R-SEC-01: why there is no new point, said as it is.
  if (!p.sim && !p.restore) notes.push(p.existing
    ? t(`Point de restauration n° ${p.existing}, déjà là (d’avant cette installation) ; chaque driver installé a aussi sa sauvegarde.`, `Restore point #${p.existing}, already there (from before this installation); each installed driver also has its backup.`)
    : p.declined ? t('Sans point de restauration : tu as dit non ; chaque driver installé a sa sauvegarde.', 'No restore point: you said no; each installed driver has its backup.')
    : t(`Point de restauration impossible${p.restoreWhy ? ` (${p.restoreWhy})` : ''} ; chaque driver installé a sa sauvegarde.`, `Restore point failed${p.restoreWhy ? ` (${p.restoreWhy})` : ''}; each installed driver has its backup.`));
  const kept = p.items.filter((it) => it.status === 'unsigned_kept');
  if (kept.length) notes.push(t(`${kept.map((it) => shortName(it.name)).join(', ')} : signature refusée et retour arrière raté. Restauration du système : Paramètres → Système → Protection du système.`, `${kept.map((it) => shortName(it.name)).join(', ')}: signature refused and rollback failed. System Restore: Settings → System → System Protection.`));
  if (p.stopped && p.left) notes.push(t(`${plural(p.left, ['élément pas commencé', 'éléments pas commencés'], ['item not started', 'items not started'])}, pas touché${agree(p.left)}.`, `${plural(p.left, ['élément pas commencé', 'éléments pas commencés'], ['item not started', 'items not started'])}, untouched.`));
  if (p.stopped === 'revert' && reboot) notes.push(t(`${p.reboot.map(shortName).join(', ')} attend le redémarrage avant de pouvoir revenir en arrière.`, `${p.reboot.map(shortName).join(', ')} waits for the restart before it can be rolled back.`));
  const line = (it, j) => {
    const [tn, , label] = result(it.status, !p.sim);
    // Applications and publishers' packages have no rollback by Drivn (`n` is null for them).
    const back = it.status === 'ok' && it.n != null && history && !history.expired;
    return ditem(it, j, it.why, html`<span class="dpill ${tn}">${label}</span>${back
      ? html`<button type="button" class="btng iconbtn sm" data-act="rollback-ask" data-pass="${p.id}" data-index="${j}" title="${t(`Revenir à ${it.from}`, `Go back to ${it.from}`)}" aria-label="${t(`Revenir à ${it.from}`, `Go back to ${it.from}`)}">${icon('undo', 16)}</button>`
      : html`<span class="iconbtn sm ghost"></span>`}`);
  };
  return modal(title, html`<div class="dend">${endBadge(tone)}
<strong class="dtitle">${title}</strong><span class="dsub">${sub}</span>
${allOk && !p.sim ? '' : chips(summary(p.items, p.sim))}</div>
${notes.map((n) => html`<span class="dnote">${n}</span>`)}
${reboot ? html`<div class="rebootbox"><span>${icon('rescan', 16)}${t(`${p.reboot.map(shortName).join(', ')} : redémarrage nécessaire. Drivn ne redémarre jamais tout seul.`, `${p.reboot.map(shortName).join(', ')}: restart needed. Drivn never restarts on its own.`)}</span>
<div class="dbtns"><button type="button" class="btng" data-act="reboot-later">${t('Plus tard', 'Later')}</button><button type="button" class="btnp" data-act="reboot-now">${t('Redémarrer maintenant', 'Restart now')}</button></div></div>` : ''}
<div class="ilist">${p.items.map(line)}</div>
<div class="dfoot"><button type="button" class="link" data-act="journal" data-from="last">${t('Journal', 'Log')}</button>
${reboot ? '' : html`<button type="button" class="btnp smallbtn" data-act="end-close">${t('Fermer', 'Close')}</button>`}</div>`, '', true);
}

function journal(s, { from }) {
  const src = from === 'live' ? s.pass : from === 'cut' ? s.interrupted : s.last;
  if (!src) return '';
  return modal(t('Journal de l’installation', 'Installation log'), html`<strong class="dtitle">${t('Journal de l’installation', 'Installation log')}</strong>
<span class="mono dnote">C:\\ProgramData\\Drivn\\passes\\${src.key ?? '…'}\\journal.jsonl</span>
<div class="jlog"${from === 'live' ? ' data-follow' : ''}>${src.log.map((l) => html`<span><span class="mono jt">${l.t}</span>${l.text}</span>`)}</div>
<span class="dnote">${t('Vue lisible du journal. Le fichier a une ligne par étape, écrite avant et après chaque action : c’est lui qui permet de revenir en arrière driver par driver et de reprendre une installation coupée.', 'Readable view of the log. The file has one line per step, written before and after each action: it is what allows rolling back driver by driver and resuming a cut installation.')}</span>
${buttons(['journal-close', t('Fermer', 'Close'), true])}`, 'journal-close', true);
}

/** The restore point alone, before an official tool (its own UAC): what runs, and since when. */
const pointRunning = (d) => html`${dhead('up', 'shield', t('Point de restauration…', 'Restore point…'), html`${t('Windows va demander l’autorisation administrateur.', 'Windows will ask for administrator permission.')}${tip(HELP.restore())}`)}
<div class="dprog">${busy(t('Windows crée le point (en général moins d’une minute)', 'Windows creates the point (usually under a minute)'), d.at)}</div>`;

// R-SEC-09: what the official tool does, what Drivn cannot do for it, and a restore point offered first (R-VRAI-05).
function tool(s, d) {
  const name = d.name;
  if (!d.run) {
    return modal(name, html`${dhead('warn', 'help', t(`${name} ne peut pas être lancé`, `${name} cannot be started`), d.why)}
<p class="dtext">${t('Drivn ne lance un outil que s’il est signé par son éditeur. Tu peux le télécharger sur la page officielle.', 'Drivn starts a tool only if its publisher signed it. You can download it from the official page.')}</p>
${buttons(['dialog-close', t('Fermer', 'Close')], ['tool-page', t('Ouvrir la page officielle', 'Open the official page'), true])}`, 'dialog-close');
  }
  const anyway = t(`Ouvrir ${name} quand même`, `Open ${name} anyway`);
  const body = {
    ask: html`${dhead('warn', 'shield', t(`Ouvrir ${name}`, `Open ${name}`), t('C’est l’outil officiel de l’éditeur qui installe, pas Drivn.', 'The publisher’s official tool installs, not Drivn.'))}
<p class="dtext">${t('Ce qu’il installe n’aura ni sauvegarde ni retour arrière par Drivn. Un point de restauration permet de remettre le PC comme avant si besoin.', 'What it installs has no backup or rollback by Drivn. A restore point can put the PC back as it was if needed.')}</p>
${buttons(['dialog-close', t('Annuler', 'Cancel')], ['tool-run', t('Ouvrir sans point', 'Open without a point')], ['tool-restore', t('Créer d’abord un point de restauration', 'Create a restore point first'), true])}`,
    restore: pointRunning(d),
    sim: html`${dhead('warn', 'help', t('Simulation : aucun point créé', 'Simulation: no point created'), t('Drivn est en mode simulation sur ce PC.', 'Drivn is in simulation mode on this PC.'))}
${buttons(['dialog-close', t('Annuler', 'Cancel')], ['tool-run', anyway, true])}`,
    restore_failed: html`${dhead('warn', 'help', t('Windows n’a pas créé le point', 'Windows did not create the point'), d.why)}
<p class="dtext">${t('Souvent : la protection du système est désactivée.', 'Often: System Protection is turned off.')}</p>
${buttons(['dialog-close', t('Annuler', 'Cancel')], ['tool-run', anyway, true])}`,
    error: html`${dhead('warn', 'help', t(`${name} n’a pas pu s’ouvrir`, `${name} could not open`), d.why)}
${buttons(['dialog-close', t('Fermer', 'Close')], ['tool-page', t('Ouvrir la page officielle', 'Open the official page'), true])}`,
  }[d.step];
  return modal(name, body, d.step === 'restore' ? '' : 'dialog-close');
}

// Update of Drivn itself (ADR-0027). R-VRAI-05: what happens, why it is safe, what is kept.
function update(s, d) {
  const pct = d.total ? (d.done * 100) / d.total : null;
  const body = {
    ask: html`${dhead('up', 'arrowup', t(`Drivn ${d.version} est disponible`, `Drivn ${d.version} is available`), t('Une nouvelle version de l’app, publiée sur GitHub.', 'A new version of the app, published on GitHub.'))}
<p class="dtext">${t('Drivn la télécharge, vérifie qu’elle vient bien de son auteur (signature), puis se réinstalle et redémarre tout seul. Ton historique et tes sauvegardes de drivers sont gardés.', 'Drivn downloads it, checks that it comes from its author (signature), then reinstalls itself and restarts. Your history and driver backups are kept.')}</p>
${buttons(['dialog-close', t('Plus tard', 'Later')], ['update-go', t('Mettre à jour', 'Update'), true])}`,
    run: html`${dhead('up', 'arrowup', t('Mise à jour…', 'Updating…'), t('Téléchargement, puis vérification de la signature. Drivn va se fermer puis revenir.', 'Download, then signature check. Drivn will close and come back.'))}
<div class="dprog">${bar(pct)}${busy(d.done ? `${size(d.done)}${d.total ? ` / ${size(d.total)} · ${Math.floor(pct)} %` : ''}` : t('Connexion à GitHub…', 'Connecting to GitHub…'), d.at)}</div>`,
    error: html`${dhead('warn', 'help', t('Mise à jour impossible', 'Update failed'), d.why)}
<p class="dtext">${t('Rien n’a été changé : tu gardes la version actuelle.', 'Nothing was changed: you keep the current version.')}</p>
${buttons(['dialog-close', t('Fermer', 'Close'), true])}`,
  }[d.step];
  return modal(t('Mise à jour', 'Update'), body, d.step === 'run' ? '' : 'dialog-close');
}

// The publisher's own installer, after Drivn's checks: NVIDIA (ADR-0030), Intel (ADR-0033). R-VRAI-05 texts.
/** « de NVIDIA », « d’Intel ». */
const de = (name) => (/^[AEIOUY]/i.test(name) ? `d’${name}` : `de ${name}`);
function pkg(s, d) {
  const anyway = t('Continuer quand même', 'Continue anyway');
  const body = {
    ask: html`${dhead('warn', 'arrowup', t(`Mettre à jour ${d.what}${d.version ? ` en ${d.version}` : ''}`, `Update ${d.what}${d.version ? ` to ${d.version}` : ''}`),
      t(`Drivn télécharge le paquet officiel ${de(d.pub)}${d.size ? ` (${d.size})` : ''}, vérifie ${d.gpu ? 'sa signature' : 'son empreinte et sa signature'}, puis ouvre l’installeur ${de(d.pub)}.`,
        `Drivn downloads ${d.pub}’s official package${d.size ? ` (${d.size})` : ''}, checks ${d.gpu ? 'its signature' : 'its hash and signature'}, then opens ${d.pub}’s installer.`))}
<p class="dtext">${t(`C’est l’installeur ${de(d.pub)} qui installe : ce qu’il change n’aura ni sauvegarde ni retour arrière par Drivn. Un point de restauration permet de remettre le PC comme avant ${d.gpu ? 'si l’écran pose problème' : 'si quelque chose ne marche plus'}.`,
  `${d.pub}’s installer does the install: what it changes has no backup or rollback by Drivn. A restore point can put the PC back as it was ${d.gpu ? 'if the screen has a problem' : 'if something stops working'}.`)}</p>
${buttons(['dialog-close', t('Annuler', 'Cancel')], ['package-go', t('Sans point', 'Without a point')], ['package-restore', t('Créer d’abord un point de restauration', 'Create a restore point first'), true])}`,
    restore: pointRunning(d),
    sim: html`${dhead('warn', 'help', t('Simulation : aucun point créé', 'Simulation: no point created'), t('Drivn est en mode simulation sur ce PC.', 'Drivn is in simulation mode on this PC.'))}
${buttons(['dialog-close', t('Annuler', 'Cancel')], ['package-go', anyway, true])}`,
    restore_failed: html`${dhead('warn', 'help', t('Windows n’a pas créé le point', 'Windows did not create the point'), d.why)}
${buttons(['dialog-close', t('Annuler', 'Cancel')], ['package-go', anyway, true])}`,
    download: html`${dhead('up', 'arrowup', t(`Téléchargement du paquet ${d.pub}…`, `Downloading ${d.pub}’s package…`), t('Ça peut prendre plusieurs minutes. Tu peux continuer à utiliser ton PC.', 'It can take several minutes. You can keep using your PC.'))}
<div class="dprog">${bar(null)}${busy(`${d.mb ?? 0} ${t('Mo', 'MB')}${d.size ? ` / ${d.size}` : ''}`, d.at)}</div>`,
    check: html`${dhead('up', 'shield', t('Vérification…', 'Checking…'), d.gpu ? t('Signature de NVIDIA et fichier complet.', 'NVIDIA’s signature and complete file.') : t(`Empreinte, signature ${de(d.pub)} et fichier complet.`, `Hash, ${d.pub}’s signature and complete file.`))}
<div class="dprog">${busy(t('Lecture du fichier et de sa signature', 'Reading the file and its signature'), d.at)}</div>`,
    launched: html`${dhead('up', 'check', t(`L’installeur ${de(d.pub)} est ouvert`, `${d.pub}’s installer is open`), d.gpu ? t('Suis ses étapes. L’écran peut clignoter pendant l’installation.', 'Follow its steps. The screen may flicker during the install.') : t('Suis ses étapes. Le réseau peut couper quelques secondes.', 'Follow its steps. The network may drop for a few seconds.'))}
<p class="dtext">${t('Ensuite, relance un scan : Drivn relira la version installée.', 'Then scan again: Drivn reads the installed version.')}</p>
${buttons(['dialog-close', t('Fermer', 'Close'), true])}`,
    error: html`${dhead('warn', 'help', t('Mise à jour impossible', 'Update failed'), d.why)}
<p class="dtext">${t('Rien n’a été installé.', 'Nothing was installed.')}</p>
${buttons(['dialog-close', t('Fermer', 'Close'), true])}`,
  }[d.step];
  return modal(d.gpu ? t('Carte graphique', 'Graphics card') : d.what, body, ['restore', 'download', 'check'].includes(d.step) ? '' : 'dialog-close');
}

/** R-SEC-01 (ADR-0036): asked before each pass; "yes" is the default button. */
function point() {
  return modal(t('Point de restauration', 'Restore point'), html`${dhead('ok', 'shield', t('Créer un point de restauration ?', 'Create a restore point?'), t('Au cas où une mise à jour se passe mal : Windows pourra tout remettre comme avant.', 'In case an update goes wrong: Windows can put everything back as it was.'))}
<span class="dnote">${t('Drivn garde ses 5 derniers points et supprime le plus ancien des siens. Il ne touche jamais aux points créés par Windows ou par quelqu’un d’autre.', 'Drivn keeps its 5 latest points and deletes the oldest of its own. It never touches points made by Windows or anyone else.')}</span>
${buttons(['point-no', t('Non, sans point', 'No, without a point')], ['point-yes', t('Oui, créer le point', 'Yes, create the point'), true])}`, 'dialog-close');
}

const DIALOGS = { point, recap, stop, rollback, working, restore, end, journal, tool, update, package: pkg };

export function overlaysView(s) {
  const d = s.dialog;
  return html`${sheetView(s)}${d ? DIALOGS[d.kind](s, d) : ''}
${s.notice ? html`<div role="status" class="glass toast" data-key="t${s.notice.id}"><span>${s.notice.text}</span>
<button type="button" class="btng closebtn" data-act="notice-close" aria-label="${t('Fermer', 'Close')}">${icon('close', 14, 2.4)}</button></div>` : ''}`;
}
