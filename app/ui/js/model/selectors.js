// Read-only queries on the state, cached so a re-render does not recompute them.
import { memo } from '../lib/store.js';
import { t, shortName } from '../lib/i18n.js';
import { sourceName } from './report.js';

const UPDATE = new Set(['update', 'update_risky']);
export const isUpdate = (x) => UPDATE.has(x.dec.state);
/** A Catalog offer the user may tick; Drivn never ticks it (R-SRC-06, R-VER-03). */
export const isInstallableAnnounce = (x) => x.dec.state === 'announced' && !!x.dec.offer?.installable;
/**
 * Updated by the publisher's own installer, opened by Drivn after its checks: the NVIDIA graphics
 * card (ADR-0030), Intel Wi-Fi and Bluetooth (ADR-0033).
 */
export const isPackage = (x) => x.dec.state === 'announced'
  && !x.dec.offer?.installable && ['nvidia', 'intel'].includes(x.dec.offer?.src);
/** Java 8 announced by Oracle: updated by Java Update, Oracle's own tool (ADR-0023, Spec - Scan complet). */
export const isJava = (x) => x.dec.state === 'announced' && x.dev.tool === 'java';
/** An application updated by winget inside Drivn's pass (ADR-0034). */
export const isApp = (x) => x.dev.tool === 'winget';
/** NVIDIA's package: run silently inside Drivn's pass (R-SEC-09, ADR-0034). */
export const isSilentPackage = (x) => isPackage(x) && x.dec.offer?.src === 'nvidia';
/** Rows updated by the publisher's own installer, opened after Drivn's pass. */
export const isOfficialTool = (x) => isPackage(x) || isJava(x);
/** The dialog of the publisher's installer for one row. */
export function packageDialog(x) {
  const src = x.dec.offer?.src;
  const gpu = src === 'nvidia';
  return { kind: 'package', id: x.dev.id, pub: sourceName(src), gpu, what: gpu ? t('la carte graphique et son audio', 'the graphics card and its audio') : shortName(x.dev.name),
    version: x.dec.offer?.vv, size: x.dec.offer?.size, step: 'ask' };
}

/** Applications and Java: their own tab; the drivers' "Tout cocher" never takes them (ADR-0035). */
export const isSoftware = (x) => isApp(x) || isJava(x);
/** Which tab a screen shows: 'apps' or 'drivers' (every other screen counts the drivers). */
export const scopeOf = (s) => (s.screen === 'apps' ? 'apps' : 'drivers');

/** Everything the screens need about the last scan result, for one tab. */
export const derive = memo((result, rows, picked, showAbsent, showAll, scope) => {
  const inScope = (x) => (scope === 'apps') === isSoftware(x);
  const items = result ? result.items.filter((x) => inScope(x) && (showAbsent || !x.dev.absent) && (showAll || !x.minor)) : [];
  const counts = { update: 0, update_risky: 0, announced: 0, up_to_date: 0, unknown: 0, no_driver: 0 };
  for (const x of items) counts[x.dec.state]++;
  // R-VRAI-04: never "all up to date" while a publisher may have newer (Spec - Sources constructeurs).
  const toCheck = items.filter((x) => x.dec.state !== 'announced' && (x.dec.unchecked.length || x.dec.hints.length)).length;
  // R-MAT-02: a plain update is ticked by default, a risky one is not.
  const isPicked = (x) => picked[x.dev.id] ?? x.dec.state === 'update';
  const pickable = items.filter((x) => (isUpdate(x) || isInstallableAnnounce(x) || isOfficialTool(x) || isApp(x)) && !rows[x.dev.id]);
  // Installed, waiting for a reboot: no longer "to update" (Spec - Installation).
  const waiting = (x) => rows[x.dev.id]?.st === 'pending';
  return {
    items,
    counts,
    isPicked,
    pickable,
    // R-SRC-06 (ADR-0034): "Tout cocher" ticks everything, no confirmation.
    bulk: pickable,
    selected: pickable.filter(isPicked),
    updates: counts.update + counts.update_risky,
    pending: items.filter((x) => x.dec.state === 'update' && !waiting(x)).length + counts.update_risky,
    unknown: counts.unknown + counts.no_driver,
    announced: counts.announced,
    // What the end of the scan announces: Windows Update and the publishers together.
    found: counts.update + counts.update_risky + counts.announced,
    toCheck,
  };
});

export const view = (s, scope = scopeOf(s)) => derive(s.result, s.rows, s.picked, s.settings.showAbsent, s.settings.showAll, scope);

/** The latest pass item that can still bring back the old driver of `devId` (R-SEC-03, R-SEC-07). */
export function rollbackTarget(history, devId) {
  for (const p of history) {
    if (p.expired) continue;
    // Applications have no rollback by Drivn (winget installs them).
    const index = p.items.findIndex((it) => it.id === devId && it.status === 'ok' && it.n != null);
    if (index >= 0) return { passId: p.id, index };
  }
  return null;
}
