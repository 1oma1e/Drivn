//! Applications with a newer version, from winget, Windows' own package manager (ADR-0034, Spec -
//! Tout mettre a jour, step 4). `winget upgrade` prints a text table; this reads it. winget itself
//! downloads and checks each installer (SHA-256 of its manifest): Drivn only runs it.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppUpdate {
    pub name: String,
    /// winget package ID (`Git.Git`): the only thing passed back to winget.
    pub id: String,
    pub version: String,
    pub available: String,
}

/// A winget package ID: `Publisher.Name[.More]`, letters, digits and `.-_+` only. Anything else
/// is never put on a command line.
pub fn valid_id(id: &str) -> bool {
    id.len() <= 128
        && id.contains('.')
        && !id.starts_with(['.', '-'])
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b".-_+".contains(&b))
}

/// The arguments that update one application, silently, without any question.
pub fn upgrade_args(id: &str) -> Option<Vec<String>> {
    valid_id(id).then(|| {
        [
            "upgrade",
            "--id",
            id,
            "--exact",
            "--source",
            "winget",
            "--silent",
            "--accept-package-agreements",
            "--accept-source-agreements",
            "--disable-interactivity",
        ]
        .map(str::to_owned)
        .to_vec()
    })
}

/// The arguments of the list (read-only, no agreement accepted).
pub const LIST_ARGS: &[&str] = &["upgrade", "--source", "winget", "--disable-interactivity"];

/// `UPDATE_INSTALL_TECHNOLOGY_MISMATCH`: the new version installs another way; tried again with
/// the old one uninstalled first.
pub const REPLACE: i32 = 0x8A15_008E_u32.cast_signed();

/// The update that uninstalls the old version first (`--uninstall-previous`, to check in the VM).
pub fn replace_args(id: &str) -> Option<Vec<String>> {
    upgrade_args(id).map(|mut args| {
        args.push("--uninstall-previous".to_owned());
        args
    })
}

/// winget's exit codes worth a second try at the end of the pass (winget-cli, returnCodes.md):
/// `INSTALL_INSTALL_IN_PROGRESS` (0x8A150102) and `INSTALL_PACKAGE_IN_USE_BY_APPLICATION` (0x8A150111).
pub const RETRY_LATER: &[i32] = &[0x8A15_0102_u32.cast_signed(), 0x8A15_0111_u32.cast_signed()];

/// What a winget exit code means, in a few words (the code stays, for a search).
pub fn why(code: i32) -> String {
    let text = match code.cast_unsigned() {
        0x8A15_0102 => "another installation was running",
        0x8A15_0111 => "the application is open: close it, then update again",
        0x8A15_008E => {
            "the new version installs another way (tried again: old one uninstalled first)"
        }
        _ => "",
    };
    let hex = format!("0x{:08X}", code.cast_unsigned());
    if text.is_empty() {
        format!("winget code {hex}")
    } else {
        format!("{text} (winget {hex})")
    }
}

/// Read only: the details of one application, with the link of its installer.
pub fn show_args(id: &str) -> Option<Vec<String>> {
    valid_id(id).then(|| {
        [
            "show",
            "--id",
            id,
            "--exact",
            "--source",
            "winget",
            "--disable-interactivity",
        ]
        .iter()
        .map(|a| (*a).to_owned())
        .collect()
    })
}

/// The installer link of `winget show` (`Installer Url: https://…`), shown before the update.
/// The key is matched loosely (it is translated on some Windows): it names the installer and a URL.
pub fn parse_installer_url(text: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let (key, value) = line.trim().split_once(": ")?;
        let key = key.to_lowercase();
        let value = value.trim();
        (key.contains("install") && key.contains("url") && value.starts_with("https://"))
            .then(|| value.to_owned())
    })
}

/// Reads the table of `winget upgrade`. Columns are found from the header line (`Name`, `Id`,
/// `Version`, `Available`); a name cut by winget ends with `…`. Lines before the header (progress
/// spinner) and after the table (counts) are ignored.
pub fn parse_upgrades(text: &str) -> Vec<AppUpdate> {
    // The spinner rewrites its line with `\r`: keep what follows the last one.
    let lines: Vec<&str> = text
        .lines()
        .map(|l| l.rsplit('\r').next().unwrap_or(l))
        .collect();
    let Some(at) = lines.iter().position(|l| {
        let w: Vec<&str> = l.split_whitespace().collect();
        w.starts_with(&["Name", "Id", "Version", "Available"])
    }) else {
        return Vec::new();
    };
    let header: Vec<char> = lines[at].chars().collect();
    let col = |name: &str| {
        let word: Vec<char> = name.chars().collect();
        (0..header.len()).find(|&i| {
            header[i..].starts_with(&word)
                && (i == 0 || header[i - 1] == ' ')
                && header.get(i + word.len()).is_none_or(|c| *c == ' ')
        })
    };
    let (Some(id_at), Some(version_at), Some(available_at)) =
        (col("Id"), col("Version"), col("Available"))
    else {
        return Vec::new();
    };
    let source_at = col("Source");
    let mut out = Vec::new();
    for line in &lines[at + 1..] {
        let chars: Vec<char> = line.chars().collect();
        if chars.iter().all(|c| *c == '-') {
            continue;
        }
        let cut = |from: usize, to: Option<usize>| -> String {
            let end = to.unwrap_or(chars.len()).min(chars.len());
            chars
                .get(from..end)
                .map(|s| s.iter().collect::<String>().trim().to_owned())
                .unwrap_or_default()
        };
        let id = cut(id_at, Some(version_at));
        let version = cut(version_at, Some(available_at));
        let available = cut(available_at, source_at);
        // The table ends at the first line that is not a row ("19 upgrades available.").
        if !valid_id(&id) || available.is_empty() || available.contains(' ') {
            break;
        }
        out.push(AppUpdate {
            name: cut(0, Some(id_at)),
            id,
            version: version.trim_start_matches('<').trim().to_owned(),
            available,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Seen on the reference PC on 2026-11-01 (cut), spinner included.
    const SEEN: &str = "   - \r   \\ \r\nName                           Id                                     Version              Available\n--------------------------------------------------------------------------------------------------------------\nDriver Booster 13              IObit.DriverBooster                    < 14.0.1.462         14.0.1.462\nJava 8 Update 501              Oracle.JavaRuntimeEnvironment          8.0.5010.8           8.0.5110.32\nPython 3.14.7 (64-bit)         Python.Python.3.14                     3.14.7               3.14.10\nVisual Studio Build Tools 2022 Microsoft.VisualStudio.2022.BuildTools 17.14.42             17.14.44\n19 upgrades available.\n3 package(s) have version numbers that cannot be determined. Use --include-unknown to see all results.\n";

    #[test]
    fn r_but_01_reads_the_winget_table() {
        let apps = parse_upgrades(SEEN);
        assert_eq!(apps.len(), 4);
        assert_eq!(apps[1].id, "Oracle.JavaRuntimeEnvironment");
        assert_eq!(apps[1].version, "8.0.5010.8");
        assert_eq!(apps[1].available, "8.0.5110.32");
        assert_eq!(apps[0].version, "14.0.1.462");
        assert_eq!(apps[2].name, "Python 3.14.7 (64-bit)");
        assert_eq!(apps[3].id, "Microsoft.VisualStudio.2022.BuildTools");
        assert!(parse_upgrades("No installed package found matching input criteria.").is_empty());
    }

    #[test]
    fn only_a_clean_id_reaches_the_command_line() {
        assert!(upgrade_args("Git.Git").is_some());
        for bad in [
            "Git",
            "Git.Git --force",
            "Git.Git;calc",
            "-x.y",
            "a.b\"",
            ".a",
        ] {
            assert!(upgrade_args(bad).is_none(), "{bad}");
        }
        let args = upgrade_args("VideoLAN.VLC").unwrap();
        assert!(args.contains(&"--silent".to_owned()) && args.contains(&"--exact".to_owned()));
    }
}
