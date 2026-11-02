// How each result of a pass is shown (Ecran - Installation, Ecran - Fin et redemarrage).
import { html } from '../lib/dom.js';
import { t } from '../lib/i18n.js';

// status -> [tone, mark, pill label, summary word (one, many)]
const RESULT = {
  ok: ['ok', '✓', () => [t('Installé', 'Installed'), t('installé', 'installed'), t('installés', 'installed')]],
  pending: ['wait', '⟳', () => [t('Après redémarrage', 'After restart'), t('après redémarrage', 'after restart'), t('après redémarrage', 'after restart')]],
  not_selected: ['mute', '–', () => [t('Pas retenu', 'Not taken'), t('pas retenu', 'not taken'), t('pas retenus', 'not taken')]],
  skipped: ['warn', '–', () => [t('Sauté', 'Skipped'), t('sauté', 'skipped'), t('sautés', 'skipped')]],
  failed: ['bad', '✕', () => [t('Échec', 'Failed'), t('échec', 'failed'), t('échecs', 'failed')]],
  unsigned: ['bad', '↩', () => [t('Remis (signature)', 'Put back (signature)'), t('remis (signature)', 'put back (signature)'), t('remis (signature)', 'put back (signature)')]],
  unsigned_kept: ['bad', '!', () => [t('Non signé, pas remis', 'Unsigned, not put back'), t('non signé, pas remis', 'unsigned, not put back'), t('non signés, pas remis', 'unsigned, not put back')]],
  sim: ['ok', '✓', () => [t('Simulé', 'Simulated'), t('simulé', 'simulated'), t('simulés', 'simulated')]],
  rolled: ['mute', '↩', () => [t('Remis comme avant', 'Put back'), t('remis comme avant', 'put back'), t('remis comme avant', 'put back')]],
  unknown: ['warn', '?', () => [t('À relire', 'To read again'), t('à relire', 'to read again'), t('à relire', 'to read again')]],
};
const ORDER = Object.keys(RESULT);

/** [tone, mark, label] of one row; a simulated install is never called "installed" (R-VRAI-01). */
export function result(st, real) {
  if (st === 'run') return ['run', '⟳', ''];
  if (st === 'queued') return ['dim', '○', t('En attente', 'Waiting')];
  if (st === 'ok' && !real) st = 'sim';
  const r = RESULT[st];
  return r ? [r[0], r[1], r[2]()[0]] : ['dim', '·', ''];
}

/** "2 installés · 1 après redémarrage · 1 sauté", from the items of a pass. */
export function summary(items, sim) {
  const n = {};
  for (const it of items) {
    const k = it.status === 'ok' && sim ? 'sim' : it.status;
    n[k] = (n[k] ?? 0) + 1;
  }
  return ORDER.filter((k) => n[k]).map((k) => {
    const [tone, mark, words] = RESULT[k];
    const [, one, many] = words();
    return { tone, text: `${mark} ${n[k]} ${n[k] > 1 ? many : one}` };
  });
}

export const chips = (list) => html`<div class="rchips">${list.map((c) => html`<span class="rchip ${c.tone}">${c.text}</span>`)}</div>`;
