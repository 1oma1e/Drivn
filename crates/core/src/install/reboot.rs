//! R-RBT-01: a restart is offered only when a result asks for it; Drivn never restarts by itself.
//! And the check that waits for it (R-SRC-04 after a restart).

use super::actions::SignerCheck;
use super::journal::{PassView, Status};
use crate::model::InstalledDriver;

/// Names of the drivers of a real pass that wait for a restart. Empty: no restart message.
pub fn reboot_needed(view: &PassView) -> Vec<String> {
    if view.simulation {
        return Vec::new();
    }
    view.items
        .iter()
        // A pending result, or an installed driver for which Windows asked for a restart.
        .filter(|i| i.status.pending_reboot() || (i.reboot_required && i.status.installed()))
        .map(|i| {
            i.name
                .clone()
                .unwrap_or_else(|| i.device_instance_id.clone())
        })
        .collect()
}

/// « Redémarrage en attente »: a pending result, and Windows last started before the pass ended.
pub fn reboot_pending(view: &PassView, last_boot: i64) -> bool {
    !reboot_needed(view).is_empty() && view.ended_at.is_some_and(|end| last_boot < end)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AfterReboot {
    /// Windows has not restarted since the pass.
    Waiting,
    /// The new driver is active and its catalog signed.
    Verified { signer: String },
    /// The new driver is active but not signed (or not checkable): rollback offered (UAC).
    Unsigned,
    /// Still not the new driver after the restart.
    NotActive,
}

/// Read-only check, at the first launch of the app after the restart.
pub fn after_reboot(
    view: &PassView,
    n: usize,
    last_boot: i64,
    now: Option<&InstalledDriver>,
    signer: impl FnOnce(&str) -> SignerCheck,
) -> Option<AfterReboot> {
    let item = view.items.iter().find(|i| i.n == n)?;
    if item.status != Status::InstalledPendingReboot || view.simulation {
        return None;
    }
    if view.ended_at.is_none_or(|end| last_boot < end) {
        return Some(AfterReboot::Waiting);
    }
    let Some(driver) = now.filter(|d| item.new.same_release(d)) else {
        return Some(AfterReboot::NotActive);
    };
    Some(
        match signer(driver.inf_path.as_deref().unwrap_or_default()) {
            SignerCheck::Signed { signer } => AfterReboot::Verified { signer },
            SignerCheck::Unsigned | SignerCheck::Unknown { .. } => AfterReboot::Unsigned,
        },
    )
}
