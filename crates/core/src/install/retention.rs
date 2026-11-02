//! R-SEC-07: Drivn keeps at most 5 restore points of its own and the backups of its 5 last passes
//! that count (ADR-0013, ADR-0038). Points and backups are pruned apart: a pass can have one
//! without the other.

use super::actions::SystemActions;
use super::journal::{Event, Line, PassView, reconstruct};
use super::run::PassStore;

/// Passes whose driver backups are kept (ADR-0013).
pub const KEEP_PASSES: usize = 5;
/// Restore points created by Drivn kept on the PC (ADR-0038).
pub const KEEP_POINTS: usize = 5;

/// Drivn's restore points beyond the 5 newest, once `current` made one, oldest last. Only a number
/// written as created in a real pass's own journal is Drivn's: a point of Windows or of anyone
/// else never is.
pub fn points_to_remove<'a>(views: &'a [PassView], current: &str) -> Vec<(&'a PassView, i64)> {
    if !views
        .iter()
        .any(|v| v.pass_id == current && !v.simulation && v.restore_seq().is_some())
    {
        return Vec::new();
    }
    let mut points: Vec<(&PassView, i64)> = views
        .iter()
        .filter(|v| !v.simulation)
        .filter_map(|v| v.restore_seq().map(|seq| (v, seq)))
        .collect();
    // Pass IDs start with their date, so their order is the order of the passes.
    points.sort_by(|a, b| b.0.pass_id.cmp(&a.0.pass_id));
    points.into_iter().skip(KEEP_POINTS).collect()
}

/// Real, finished, with at least one driver backup, not pruned yet.
pub fn counts(view: &PassView) -> bool {
    !view.simulation
        && !view.interrupted()
        && !view.pruned
        && view.items.iter().any(|i| i.backup.is_some())
}

/// The passes whose backups go after `current`. Nothing unless the new pass is real, finished and
/// made its restore point: the old safety nets stay until a new one exists (ADR-0013).
pub fn to_prune<'a>(views: &'a [PassView], current: &str) -> Vec<&'a PassView> {
    if !views.iter().any(|v| {
        v.pass_id == current && !v.simulation && !v.interrupted() && v.restore_seq().is_some()
    }) {
        return Vec::new();
    }
    let mut counting: Vec<&PassView> = views.iter().filter(|v| counts(v)).collect();
    counting.sort_by(|a, b| b.pass_id.cmp(&a.pass_id));
    counting.into_iter().skip(KEEP_PASSES).collect()
}

/// Removes Drivn's oldest restore points and the backups of its oldest passes; each pass's journal
/// says what was removed. The removal is always tried: Windows' list can lag behind (seen on
/// 2026-11-02). Only a removal that fails **and** a number absent from the list mean the point is
/// already gone (Windows deleted it for disk space): noted so, never asked again. Any other failure
/// is retried at the next pass.
pub fn prune(store: &mut dyn PassStore, sys: &mut dyn SystemActions, current: &str) {
    let views: Vec<PassView> = store
        .passes()
        .iter()
        .filter_map(|(_, text)| reconstruct(text).ok())
        .collect();
    for (view, seq) in points_to_remove(&views, current) {
        let (ok, error) = match sys.remove_restore_point(seq) {
            Ok(()) => (true, None),
            Err(e) => {
                // An unreadable list proves nothing: retried later.
                let gone = sys.restore_points().is_ok_and(|l| !l.contains(&seq));
                (
                    gone,
                    Some(if gone {
                        format!("already gone ({e})")
                    } else {
                        e
                    }),
                )
            }
        };
        let line = Line {
            at: sys.now(),
            simulation: false,
            event: Event::PointRemoved { seq, ok, error },
        };
        let _ = store.append(&view.pass_id, &line);
    }
    for view in to_prune(&views, current) {
        let error = sys.remove_backup(&view.pass_id).err();
        let line = Line {
            at: sys.now(),
            simulation: false,
            event: Event::Pruned {
                ok: error.is_none(),
                error,
            },
        };
        let _ = store.append(&view.pass_id, &line);
    }
}
