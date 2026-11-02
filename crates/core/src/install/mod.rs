//! Install pass logic (docs/specs/Spec - Installation.md). Everything here is pure or goes through
//! [`actions::SystemActions`], so each rule is tested with a fake system: no test touches Windows.
//! The elevated `drivn-installer` gives the real (and the simulated) system; the app only reads.

pub mod actions;
pub mod control;
pub mod journal;
pub mod official;
pub mod plan;
pub mod reboot;
pub mod retention;
pub mod run;
pub mod setup;
