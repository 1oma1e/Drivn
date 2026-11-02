//! Argument parsing of `drivn-installer.exe`, apart from the binary: the binary carries the
//! `requireAdministrator` manifest, so its own test harness could not run without the UAC.

#[derive(Debug, PartialEq, Eq)]
pub enum Task {
    Pass {
        pass_id: String,
        sha256: String,
    },
    Resume {
        pass_id: String,
        stop: Option<drivn_core::install::control::StopMode>,
    },
    Rollback {
        pass_id: String,
        n: usize,
    },
}

impl Task {
    pub fn pass_id(&self) -> &str {
        match self {
            Self::Pass { pass_id, .. }
            | Self::Resume { pass_id, .. }
            | Self::Rollback { pass_id, .. } => pass_id,
        }
    }
}

/// A GUID, nothing else: the update ID goes into a WUA search criteria.
pub fn is_guid(text: &str) -> bool {
    text.len() == 36
        && text.bytes().enumerate().all(|(i, b)| match i {
            8 | 13 | 18 | 23 => b == b'-',
            _ => b.is_ascii_hexdigit(),
        })
}

pub fn parse_args(args: &[String]) -> Result<Task, String> {
    use drivn_core::install::control::StopMode;
    use drivn_core::install::plan::valid_pass_id;
    let task = match args {
        [flag, id, sha_flag, sha] if flag == "--pass" && sha_flag == "--sha256" => Task::Pass {
            pass_id: id.clone(),
            sha256: sha.clone(),
        },
        [flag, id] if flag == "--resume" => Task::Resume {
            pass_id: id.clone(),
            stop: None,
        },
        [flag, id, stop_flag, mode] if flag == "--resume" && stop_flag == "--stop" => {
            Task::Resume {
                pass_id: id.clone(),
                stop: Some(match mode.as_str() {
                    "keep" => StopMode::Keep,
                    "revert" => StopMode::Revert,
                    _ => return Err(format!("unknown stop mode {mode}")),
                }),
            }
        }
        [flag, id, n] if flag == "--rollback" => Task::Rollback {
            pass_id: id.clone(),
            n: n.parse().map_err(|_| format!("bad driver number {n}"))?,
        },
        _ => return Err("usage: --pass <id> --sha256 <hash> | --resume <id> [--stop keep|revert] | --rollback <id> <n>".to_owned()),
    };
    let (Task::Pass { pass_id, .. }
    | Task::Resume { pass_id, .. }
    | Task::Rollback { pass_id, .. }) = &task;
    if !valid_pass_id(pass_id) {
        return Err("invalid pass id".to_owned());
    }
    Ok(task)
}

/// Exit codes the app reads.
pub mod exit {
    pub const BAD_ARGS: u8 = 2;
    pub const ALREADY_RUNNING: u8 = 3;
    pub const REFUSED: u8 = 4;
}

#[cfg(test)]
mod tests {
    use super::{Task, parse_args};

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn r_sec_05_installer_takes_no_path_and_checks_the_pass_id() {
        assert!(parse_args(&args(&["--pass", "2026-11-01T1410-a3f9", "--sha256", "ab"])).is_ok());
        assert!(parse_args(&args(&["--pass", "..\\..\\Windows", "--sha256", "ab"])).is_err());
        assert!(parse_args(&args(&["--pass", "C:\\plan.json", "--sha256", "ab"])).is_err());
        assert!(parse_args(&args(&["--plan", "C:\\plan.json"])).is_err());
        assert_eq!(
            parse_args(&args(&["--rollback", "2026-11-01T1410-a3f9", "2"])),
            Ok(Task::Rollback {
                pass_id: "2026-11-01T1410-a3f9".to_owned(),
                n: 2
            })
        );
        assert!(
            parse_args(&args(&[
                "--resume",
                "2026-11-01T1410-a3f9",
                "--stop",
                "force"
            ]))
            .is_err()
        );
    }

    #[test]
    fn r_sec_05_update_id_is_a_guid_and_nothing_else() {
        assert!(super::is_guid("0b5c0a7e-1d2f-4a3b-9c8d-7e6f5a4b3c2d"));
        assert!(!super::is_guid("x' or UpdateID='y"));
        assert!(!super::is_guid(
            "0b5c0a7e-1d2f-4a3b-9c8d-7e6f5a4b3c2d' or 1=1"
        ));
        assert!(!super::is_guid(""));
    }

    /// R-DRT-01: only the installer asks for administrator rights; the app never does.
    #[test]
    fn r_drt_01_only_the_installer_requires_administrator() {
        let build = include_str!("../build.rs");
        assert!(build.contains("/MANIFESTUAC:level='requireAdministrator'"));
        for app_file in [
            include_str!("../../../app/src-tauri/build.rs"),
            include_str!("../../../app/src-tauri/tauri.conf.json"),
            include_str!("../../../app/src-tauri/Cargo.toml"),
            include_str!("../../../app/src-tauri/src/main.rs"),
            include_str!("../../../app/src-tauri/src/pass.rs"),
        ] {
            assert!(!app_file.contains("requireAdministrator"));
            assert!(!app_file.contains("highestAvailable"));
        }
    }
}
