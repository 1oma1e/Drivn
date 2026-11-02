//! Orders the app writes in `control\<passe>.cmd` (Spec - Installation, « Ordres », R-SEC-08).
//! One word, read only between two drivers and while waiting; a word out of its state is ignored.

use serde::{Deserialize, Serialize};

/// The control file is never read past this size.
pub const MAX_CONTROL_BYTES: u64 = 32;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Order {
    Run,
    Pause,
    CancelKeep,
    CancelRevert,
    ContinueNoRestore,
}

impl Order {
    pub fn word(self) -> &'static str {
        match self {
            Self::Run => "run",
            Self::Pause => "pause",
            Self::CancelKeep => "cancel_keep",
            Self::CancelRevert => "cancel_revert",
            Self::ContinueNoRestore => "continue_no_restore",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        [
            Self::Run,
            Self::Pause,
            Self::CancelKeep,
            Self::CancelRevert,
            Self::ContinueNoRestore,
        ]
        .into_iter()
        .find(|o| o.word() == raw.trim())
    }
}

/// Where the pass is when it reads the control file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Waiting {
    Running,
    Paused,
    RestoreConsent,
}

/// The table of the spec: each word is accepted only in its state.
pub fn accepted(order: Order, state: Waiting) -> bool {
    matches!(
        (order, state),
        (Order::Run, Waiting::Paused)
            | (Order::Pause, Waiting::Running)
            | (
                Order::CancelKeep | Order::CancelRevert,
                Waiting::Running | Waiting::Paused | Waiting::RestoreConsent
            )
            | (Order::ContinueNoRestore, Waiting::RestoreConsent)
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StopMode {
    /// Keep what is installed.
    Keep,
    /// Put back every driver installed by the pass, last first.
    Revert,
}
