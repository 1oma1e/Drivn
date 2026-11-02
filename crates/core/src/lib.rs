//! Pure logic of Drivn. Nothing here talks to Windows: the Windows side lives in `drivn-windows`
//! behind the [`scan::DeviceSource`] and [`scan::UpdateSource`] traits, so every rule can be tested
//! on captured snapshots (docs/TESTS.md).

pub mod anonymize;
pub mod decide;
pub mod games;
pub mod install;
pub mod model;
pub mod scan;
pub mod sources;
pub mod summary;
pub mod time;
