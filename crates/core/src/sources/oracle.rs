//! Oracle's Java Update version file (Spec - Sources constructeurs, 4.4; format seen on
//! 2026-11-01): one version per line, Java 8 written `1.8.0_<update>`.

use super::hosts::{HttpsUrl, UrlRefused};

pub fn baseline_url() -> Result<HttpsUrl, UrlRefused> {
    HttpsUrl::parse("https://javadl-esd-secure.oracle.com/update/baseline.version")
}

/// The latest Java 8 update number (`1.8.0_511` → 511). `None`: not in the file.
pub fn latest_java8_update(text: &str) -> Option<u32> {
    text.lines()
        .filter_map(|line| line.trim().strip_prefix("1.8.0_"))
        .filter_map(|n| n.parse::<u32>().ok())
        .max()
}

/// Update number of an installed Java 8 from its `DisplayVersion` (`8.0.5110.9` → 511).
pub fn installed_java8_update(display_version: &str) -> Option<u32> {
    let mut parts = display_version.trim().split('.');
    let (major, minor, third) = (parts.next()?, parts.next()?, parts.next()?);
    if major != "8" || minor != "0" {
        return None;
    }
    let third: u32 = third.parse().ok()?;
    third.is_multiple_of(10).then_some(third / 10)
}
