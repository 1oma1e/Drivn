//! Sources other than Windows Update (Spec - Sources constructeurs): the hosts Drivn may call,
//! how it calls them (R-SRC-01, R-SRC-11), and pure readers of what they answer. The network
//! itself is behind [`http::Http`], so every rule is tested without it.

pub mod archive;
pub mod asus;
pub mod guard;
pub mod hosts;
pub mod http;
pub mod intel;
pub mod ms_catalog;
pub mod nvidia;
pub mod oracle;
pub mod pe;
pub mod text;
pub mod tools;
pub mod vendors;
pub mod winget;

use serde::{Deserialize, Serialize};

/// The fragile sources of phase 3 that Drivn queries itself (docs/SOURCES.md).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceId {
    MsCatalog,
    Nvidia,
    Oracle,
    Asus,
    Intel,
}

impl SourceId {
    pub fn key(self) -> &'static str {
        match self {
            Self::MsCatalog => "ms_catalog",
            Self::Nvidia => "nvidia",
            Self::Oracle => "oracle",
            Self::Asus => "asus",
            Self::Intel => "intel",
        }
    }

    /// R-SRC-06: an offer of a fragile source is never ticked by Drivn.
    pub fn fragile(self) -> bool {
        true
    }
}
