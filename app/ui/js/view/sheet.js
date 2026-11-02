// Driver sheet (drawer on the right): everything the decision used, so it can be checked.
import { html } from '../lib/dom.js';
import { t, shortDate, shortName, sep } from '../lib/i18n.js';
import { icon, deviceIcon } from './icons.js';
import { rollbackTarget, isPackage } from '../model/selectors.js';
import { sourceName, versions, groupLabel } from '../model/report.js';
import { tip, busy, HELP } from './parts.js';

const RISK = {
  less_precise: () => t('Plus générique que ton driver actuel : il peut retirer des réglages du constructeur.', 'More generic than your current driver: it may remove the maker’s settings.'),
  other_provider: () => t('Éditeur différent du driver actuel : on ne devine pas qu’ils sont pareils.', 'Another publisher than the current driver: Drivn does not guess they are the same.'),
  cannot_judge: () => t('On ne sait pas quel ID a servi au driver actuel : impossible de comparer.', 'The ID used by the current driver is unknown: no comparison possible.'),
  no_installed_driver: () => t('Aucun driver installé : précision impossible à comparer.', 'No driver installed: precision cannot be compared.'),
};
const WHQL = 'Hardware Compatibility';
/** A driver installed from an Intel package (`oem#.inf`): Intel's site refuses Drivn, Intel DSA can check it (ADR-0033). */
const intelDriver = (d) => !d.tool && !d.bios && /intel/i.test(d.inst?.p ?? '') && /^oem\d+\.inf$/i.test(d.inst?.inf ?? '');

function checks(dec, inst, d) {
  const out = [];
  const add = (ok, text) => out.push([ok, text]);
  const o = dec.offer;
  const fragile = (name) => t(`Source officielle mais non documentée : ${name}`, `Official but undocumented source: ${name}`);
  if (d.bios) {
    add(true, t(`Version lue sur le site de ${inst?.p}`, `Version read on ${inst?.p}’s site`));
    add(false, fragile(inst?.p));
    add(false, t('Jamais installé par Drivn', 'Never installed by Drivn'));
  } else if (o && d.tool === 'java') {
    add(true, t('Plus récent, selon le fichier de versions d’Oracle', 'Newer, according to Oracle’s version file'));
    add(false, fragile('Oracle'));
    add(true, t('Installé par Java Update, l’outil d’Oracle, après vérification de sa signature', 'Installed by Java Update, Oracle’s tool, after its signature is checked'));
  } else if (o && d.tool === 'winget') {
    add(true, t('Plus récent, selon winget', 'Newer, according to winget'));
    add(true, t('Téléchargé chez l’éditeur, empreinte vérifiée par winget', 'Downloaded from the publisher, hash checked by winget'));
  } else if (o) {
    add(true, t('Fait pour ton matériel (même ID)', 'Made for your hardware (same ID)'));
    if (inst) {
      const r = dec.risk;
      add(r !== 'less_precise' && r !== 'cannot_judge', r === 'less_precise' ? t('Plus générique que l’actuel', 'More generic than the current one') : r === 'cannot_judge' ? t('Précision impossible à comparer', 'Precision cannot be compared') : t('Aussi précis que l’actuel', 'As precise as the current one'));
      add(r !== 'other_provider' && r !== 'cannot_judge', `${r === 'other_provider' ? t('Autre éditeur', 'Another publisher') : t('Même éditeur', 'Same publisher')}${sep()}${o.p}`);
      add(true, o.src ? t('Plus récent, selon le site (confirmé en ouvrant le paquet)', 'Newer, according to the site (confirmed by opening the package)') : t('Plus récent, en version et en date', 'Newer, in version and date'));
    }
    if (o.src) add(false, fragile(sourceName(o.src)));
    else add(true, t('Source officielle : Windows Update', 'Official source: Windows Update'));
  } else if (dec.state === 'up_to_date') {
    add(true, t('Windows Update n’a rien de plus récent', 'Windows Update has nothing newer'));
    if (inst?.signer) add(true, inst.signer.includes(WHQL) ? t('Signé et certifié par Microsoft (WHQL)', 'Signed and certified by Microsoft (WHQL)') : t(`Signé par ${inst.signer}`, `Signed by ${inst.signer}`));
    for (const name of dec.unchecked) add(false, t(`Pas pu vérifier chez ${name}`, `Could not check at ${name}`));
  } else add(false, dec.reason);
  return out;
}

function why(dec, r, d) {
  if (r.why) return t(`Dernière installation : ${r.why}.`, `Last installation: ${r.why}.`);
  switch (dec.state) {
    case 'update': return t('Windows Update a une version plus récente, faite pour ce matériel. Son fichier et sa signature seront vérifiés à l’installation.', 'Windows Update has a newer version, made for this hardware. Its file and signature are checked at install time.');
    case 'update_risky': return t('Il y a plus récent, mais un point nous empêche de le conseiller. À toi de voir.', 'There is a newer one, but one point stops Drivn from recommending it. Your call.');
    case 'announced': {
      const o = dec.offer;
      if (d.bios) return t(`${sourceName(o.src)} publie le BIOS ${o.v}${o.d ? ` (${shortDate(o.d)})` : ''}. Drivn ne met jamais à jour un BIOS : une coupure pendant la mise à jour peut empêcher le PC de démarrer. Si tu veux le faire, suis la page support de ta carte mère.`,
        `${sourceName(o.src)} publishes BIOS ${o.v}${o.d ? ` (${shortDate(o.d)})` : ''}. Drivn never updates a BIOS: a power cut during the update can stop the PC from starting. If you want to, follow your motherboard’s support page.`);
      if (d.tool === 'java') return t(`Oracle annonce Java ${o.v}. Java n’est pas un driver : c’est Java Update, l’outil officiel d’Oracle, qui l’installe. Drivn vérifie d’abord sa signature et te propose un point de restauration.`,
        `Oracle announces Java ${o.v}. Java is not a driver: Java Update, Oracle’s official tool, installs it. Drivn first checks its signature and offers a restore point.`);
      if (d.tool === 'winget') return t(`Une version plus récente (${o.v}) existe. winget, l’outil de Microsoft intégré à Windows, la télécharge chez l’éditeur et vérifie son empreinte, dans la même installation que les drivers.`,
        `A newer version (${o.v}) exists. winget, Microsoft’s tool built into Windows, downloads it from the publisher and checks its hash, in the same installation as the drivers.`);
      const name = sourceName(o.src);
      const by = o.pub ? t(` (déposé par ${o.pub})`, ` (uploaded by ${o.pub})`) : '';
      if (o.installable) return t(`${name} annonce une version plus récente${by}. Windows Update ne la propose pas à ce PC : elle est peut-être prévue pour d’autres modèles. Si tu l’installes, Drivn vérifie le fichier et sa signature, garde une copie de ton driver actuel et refuse si la version n’est pas vraiment plus récente.`,
        `${name} announces a newer version${by}. Windows Update does not offer it to this PC: it may be meant for other models. If you install it, Drivn checks the file and its signature, keeps a copy of your current driver and refuses if the version is not really newer.`);
      if (o.src === 'intel') return t(`${name} annonce une version plus récente. Drivn télécharge le paquet officiel, vérifie son empreinte (celle de la page d’Intel) et sa signature, puis ouvre l’installeur d’Intel.`,
        `${name} announces a newer version. Drivn downloads the official package, checks its hash (the one on Intel’s page) and its signature, then opens Intel’s installer.`);
      return t(`${name} annonce une version plus récente. Drivn ne l’installe pas pièce par pièce (un driver graphique à moitié installé peut laisser l’écran noir) : il télécharge le paquet officiel, vérifie sa signature et lance l’installeur de ${name}.`,
        `${name} announces a newer version. Drivn does not install it piece by piece (a half-installed graphics driver can leave a black screen): it downloads the official package, checks its signature and runs ${name}’s installer.`);
    }
    case 'up_to_date':
      if (d.bios) return t(`Tu as le dernier BIOS publié par ${d.inst?.p}.`, `You have the latest BIOS published by ${d.inst?.p}.`);
      if (dec.hints[0]?.kind === 'publisher_lists_newer') return t(`Windows Update n’a rien de plus récent. Le fabricant de ta carte mère publie « ${dec.hints[0].text} ». Sa liste ne dit pas pour quel matériel exact : c’est un indice, pas une mise à jour. Tu peux le récupérer sur la page support de ta carte mère.`,
        `Windows Update has nothing newer. Your motherboard’s maker publishes “${dec.hints[0].text}”. Its list does not say for which exact hardware: it is a hint, not an update. You can get it from your motherboard’s support page.`);
      if (dec.hints.length) return t(`Windows Update n’a rien de plus récent. Le dernier paquet du fabricant contient peut-être plus récent (${dec.hints[0].text}) : on ne le saura qu’en l’ouvrant.`,
        `Windows Update has nothing newer. The maker’s latest package may hold a newer one (${dec.hints[0].text}): it is only known by opening it.`);
      if (dec.unchecked.length) return t(`Windows Update n’a rien de plus récent. Drivn n’a pas pu vérifier chez ${dec.unchecked.join(', ')} : il ne dit donc pas « à jour » tout court.`,
        `Windows Update has nothing newer. Drivn could not check at ${dec.unchecked.join(', ')}: so it does not just say “up to date”.`);
      return t('Tu as déjà le plus récent que Windows Update propose pour ce matériel.', 'You already have the newest Windows Update offers for this hardware.');
    case 'no_driver': return t('Aucun driver installé, et Windows Update n’en propose pas.', 'No driver installed, and Windows Update offers none.');
    default: return t(`${dec.reason} Drivn préfère dire « inconnu » que deviner.`, `${dec.reason} Drivn would rather say “unknown” than guess.`);
  }
}

export function sheetView(s) {
  const x = s.result?.items.find((it) => it.dev.id === s.sel);
  if (!x) return '';
  const { dev: d, dec } = x;
  const r = s.rows[d.id] ?? {};
  const done = r.st === 'ok';
  const inst = d.inst;
  const o = dec.offer;
  const isUp = dec.state === 'update';
  const back = done && rollbackTarget(s.history, d.id);
  const newTone = done ? 'ok' : o ? (isUp ? 'up' : 'risk') : '';
  const appUrl = s.appUrls?.[d.id];
  const lines = [
    [t('Éditeur', 'Publisher'), inst?.p], [html`${t('Signature', 'Signature')}${tip(HELP.signature())}`, inst?.signer], [html`${t('Fichier INF', 'INF file')}${tip(HELP.inf())}`, inst?.inf],
    [html`${t('ID utilisé', 'ID used')}${tip(HELP.hardwareId())}`, inst?.match],
    // Exactly where it is downloaded from, before anything is installed.
    [t('Téléchargé depuis', 'Downloaded from'), d.tool === 'winget' ? (appUrl ?? busy(t('lien demandé à winget…', 'asking winget for the link…')))
      : o?.url ?? (o && !o.src ? t('Windows Update (serveurs Microsoft), lien donné par Windows au téléchargement', 'Windows Update (Microsoft servers), link given by Windows at download time') : null)],
    [t('ID proposé', 'Offered ID'), o?.hid], [t('Source', 'Source'), o?.src ? `${sourceName(o.src)}${o.vv ? ` · ${o.vv}` : ''}${o.size ? ` · ${o.size}` : ''}` : o ? 'Windows Update' : ''],
    ['Hardware IDs', d.hw.join(' · ')], ['Compatible IDs', d.compat.join(' · ')],
    [t('Écartés', 'Set aside'), dec.rejected.join(' · ') || t('aucun', 'none')],
  ];
  const disabled = s.pass ? 'disabled' : '';

  return html`<div class="overlay right">
<button type="button" class="backdrop" data-act="close-sheet" aria-label="${t('Fermer la fiche', 'Close the sheet')}"></button>
<aside role="dialog" aria-label="${shortName(d.name)}" class="glass drawer">
<div class="dtop"><span class="dico">${icon(deviceIcon(d), 26, 1.7)}</span>
<div class="dname"><h2>${shortName(d.name)}</h2><span>${groupLabel(d.group)}${inst?.p ? ` · ${inst.p}` : ''}</span></div>
<button type="button" class="btng closebtn" data-act="close-sheet" aria-label="${t('Fermer', 'Close')}">${icon('close', 16, 2.4)}</button></div>
${d.role ? html`<p class="role">${d.role}</p>` : ''}
<p class="why">${why(dec, r, d)}</p>
${o && !done && !d.bios ? html`<p class="role">${t('Une version plus récente corrige en général des bugs, la stabilité ou la compatibilité. Le fabricant ne publie pas toujours le détail : Drivn ne l’invente pas.', 'A newer version usually fixes bugs, stability or compatibility. The maker does not always publish the details: Drivn does not make them up.')}</p>` : ''}
<div class="vs">
<div class="vcard"><span class="k">${t('Actuel', 'Current')}</span><span class="mono v">${o ? versions(inst, o).cur : inst ? inst.v : t('aucun', 'none')}</span><span class="d">${inst ? shortDate(inst.d) : '—'}</span></div>
<span class="varrow">${icon('arrow', 18, 2.4)}</span>
<div class="vcard ${newTone}"><span class="k">${t('Nouveau', 'New')}</span>
<span class="mono v">${o ? versions(inst, o).nw : dec.state === 'up_to_date' ? t('rien', 'nothing') : '?'}</span>
<span class="d">${o ? `${shortDate(o.d)} · ${o.src ? t(`annoncée par ${sourceName(o.src)}`, `announced by ${sourceName(o.src)}`) : 'Windows Update'}` : dec.state === 'up_to_date' ? t('de plus récent', 'newer') : t('on ne sait pas', 'unknown')}</span></div>
</div>
<div class="checks">${checks(dec, inst, d).map(([ok, text], i) => html`<span class="ck" style="animation-delay:${150 + i * 90}ms"><span class="ckm ${ok ? 'ok' : 'no'}">${ok ? '✓' : '!'}</span>${text}</span>`)}</div>
${dec.risk && !done ? html`<span class="warnbox">${RISK[dec.risk]()}${tip(HELP.risky())}</span>` : ''}
${dec.state === 'announced' && !d.tool && !d.bios ? html`<span class="role">${t('Version annoncée par le site du fabricant', 'Version announced by the maker’s site')}${tip(HELP.announced())}</span>` : ''}
<div class="tech${s.techOpen ? ' open' : ''}">
<button type="button" class="techhead" data-act="tech" aria-expanded="${!!s.techOpen}">${t('Détails techniques', 'Technical details')} ${icon('chevron', 14)}</button>
${s.techOpen ? html`<div class="techgrid">${lines.map(([k, val]) => html`<span>${k}</span><span class="mono">${val || '—'}</span>`)}</div>` : ''}
</div>
${isPackage(x) && !r.st ? html`<button type="button" class="btnw sheetbtn" data-act="package-ask" data-id="${d.id}" ${disabled}>${t(`Mettre à jour avec l’installeur ${sourceName(o.src)}`, `Update with ${sourceName(o.src)}’s installer`)}</button>` : ''}
${intelDriver(d) && !r.st ? html`<button type="button" class="btng sheetbtn" data-act="tool-ask" data-tool="intel_dsa" ${disabled}>${t('Vérifier avec Intel DSA', 'Check with Intel DSA')}</button>` : ''}
${d.tool === 'java' && o && !r.st ? html`<button type="button" class="btnw sheetbtn" data-act="tool-ask" data-tool="java" ${disabled}>${t('Mettre à jour avec Java Update', 'Update with Java Update')}</button>` : ''}
${o && d.tool === 'winget' && !r.st ? html`<button type="button" class="btnw sheetbtn" data-act="install-one" data-id="${d.id}" ${disabled}>${t('Mettre à jour', 'Update')}</button>` : ''}
${o && !d.tool && (!o.src || o.installable) && !r.st ? html`<button type="button" class="${isUp ? 'btnp' : 'btnw'} sheetbtn" data-act="install-one" data-id="${d.id}" ${disabled}>${isUp ? t('Mettre à jour', 'Update') : o.src ? t('Installer (source non documentée)', 'Install (undocumented source)') : t('Installer quand même', 'Install anyway')}</button>` : ''}
${back ? html`<button type="button" class="btng sheetbtn" data-act="rollback-ask" data-pass="${back.passId}" data-index="${back.index}" ${disabled}>${icon('undo', 16)}${t('Revenir à l’ancien driver', 'Go back to the old driver')}</button>` : ''}
</aside></div>`;
}
