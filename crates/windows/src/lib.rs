//! Read-only Windows adapters. Nothing in this crate installs, removes or changes anything:
//! it reads devices, driver properties, what Windows Update offers and file security, and starts
//! the elevated installer (which makes every change, after its own checks).
//! `unsafe` is allowed here only, around Win32 calls (ADR-0004).
#![cfg(windows)]
#![allow(unsafe_code)]

pub mod authenticode;
mod devices;
pub mod http;
pub mod inf;
pub mod installer;
mod programs;
pub mod security;
mod signatures;
pub mod vendors;
mod windows_update;
pub mod winget;

pub use devices::{CfgMgrDevices, read_one};
pub use programs::{board, installed_programs, windows_release};
pub use signatures::inf_signer;
pub use windows_update::WindowsUpdateAgent;
