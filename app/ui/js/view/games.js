// Game components screen (ADR-0016): software games need, read from the registry. Installed version
// only: the latest one is unknown until its source is checked (R-VRAI-04).
import { html } from '../lib/dom.js';
import { t, plural, s as agree } from '../lib/i18n.js';
import { icon } from './icons.js';
import { java8Update } from '../model/report.js';

const KINDS = [
  ['visual_cpp', 'Visual C++'], ['dot_net', '.NET'], ['java', 'Java'], ['phys_x', 'PhysX'],
  ['vulkan', 'Vulkan'], ['open_al', 'OpenAL'], ['xna', 'XNA Framework'], ['game_input', 'GameInput'],
];

/** Java 8 only: Oracle's version file. Never "up to date" without it. */
function latest(g, java) {
  const update = g.kind === 'java' ? java8Update(g.version) : null;
  if (update == null || !java?.update) return t('dernière : inconnue', 'latest: unknown');
  return update >= java.update ? t('à jour selon Oracle', 'up to date according to Oracle') : t(`dernière : 8 Update ${java.update} (Oracle)`, `latest: 8 Update ${java.update} (Oracle)`);
}

export function gamesView(s) {
  const games = s.result?.games ?? [];
  const groups = KINDS.map(([kind, label]) => [label, games.filter((g) => g.kind === kind)]).filter(([, list]) => list.length);
  return html`<section class="page rise" data-key="games">
<h1>${t('Jeux', 'Games')}</h1>
<span class="lead">${games.length ? t(`${plural(games.length, ['composant', 'composants'], ['component', 'components'])} de jeu lu${agree(games.length)} sur ton PC. `, `${plural(games.length, ['composant', 'composants'], ['component', 'components'])} for games found on your PC. `) : ''}${t('Ce sont des logiciels, pas des drivers.', 'These are software, not drivers.')}</span>
${groups.length ? html`<div class="plist">${groups.map(([label, list]) => html`<div class="pcard" data-key="g${label}">
<div class="phead static"><span class="ico">${icon('game', 18, 1.8)}</span><b>${label}</b><span class="pmeta">${plural(list.length, ['version', 'versions'], ['version', 'versions'])}</span></div>
<div class="pitems">${list.map((g) => html`<div class="pitem">
<span class="pname">${g.name}<small>${g.publisher ?? t('éditeur inconnu', 'unknown publisher')}</small></span>
<span class="mono pver">${g.version ?? '?'}</span>
<span class="pstat">${latest(g, s.result?.java)}</span>
${g.kind === 'java' ? html`<button type="button" class="btng smallbtn" data-act="tool-ask" data-tool="java">${t('Vérifier avec Java Update', 'Check with Java Update')}</button>` : ''}
</div>`)}</div></div>`)}</div>`
  : html`<div class="empty"><span class="emptyico">${icon('game', 26, 2)}</span>${t('Aucun composant de jeu trouvé sur ce PC.', 'No game component found on this PC.')}</div>`}
<div class="pfoot"><span>${t('« inconnue » : la source officielle n’est pas vérifiée. Java : lu dans le fichier de versions d’Oracle, une source non documentée. DirectX fait partie de Windows : Windows Update le met à jour.',
    '“unknown”: the official source is not checked. Java: read in Oracle’s version file, an undocumented source. DirectX is part of Windows: Windows Update updates it.')}</span></div>
</section>`;
}
