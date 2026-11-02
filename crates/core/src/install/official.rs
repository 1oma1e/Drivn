//! Publishers' own installers run inside Drivn's elevated pass, silently (R-SEC-09, ADR-0034).
//! The app downloads and checks the package; the installer copies it into its own folder, checks
//! its signature again on that copy and only then runs it. Nothing from the app is believed
//! (R-SEC-05): the source names one row of this table, the file must look like the app's cache.

/// One publisher whose installer may run in the pass.
#[derive(Debug)]
pub struct Official {
    /// `SourceId::key()` of the source that announced the version.
    pub source: &'static str,
    /// R-SRC-03: Authenticode signer of the package, checked on the installer's own copy.
    pub signer: &'static str,
    /// Folder of the app's cache: `%LocalAppData%\Drivn\cache\<folder>\`.
    pub folder: &'static str,
    /// Silent switches. NVIDIA: `-s -noreboot` (to be checked in the VM, docs/TESTS.md).
    pub args: &'static [&'static str],
    /// Longest run before the installer is stopped.
    pub max_minutes: u64,
}

/// Intel is not here: its source is off (ADR-0033) and its silent switch is not known.
pub const OFFICIAL: &[Official] = &[Official {
    source: "nvidia",
    signer: "NVIDIA Corporation",
    folder: "nvidia",
    args: &["-s", "-noreboot"],
    max_minutes: 45,
}];

pub fn official(source: &str) -> Option<&'static Official> {
    OFFICIAL.iter().find(|o| o.source == source)
}

/// `X:\...\Drivn\cache\<folder>\<name>.exe`, with a plain file name and no `..`.
pub fn valid_file(official: &Official, file: &str) -> bool {
    let parts: Vec<&str> = file.split('\\').collect();
    let drive = parts.first().is_some_and(|d| {
        let b = d.as_bytes();
        b.len() == 2 && b[0].is_ascii_alphabetic() && b[1] == b':'
    });
    let n = parts.len();
    if !drive
        || n < 5
        || parts
            .iter()
            .any(|p| p.is_empty() || *p == "." || *p == "..")
    {
        return false;
    }
    let name = parts[n - 1];
    let plain = !name.starts_with('.')
        && name.to_ascii_lowercase().ends_with(".exe")
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b));
    plain
        && parts[n - 2].eq_ignore_ascii_case(official.folder)
        && parts[n - 3].eq_ignore_ascii_case("cache")
        && parts[n - 4].eq_ignore_ascii_case("Drivn")
}
