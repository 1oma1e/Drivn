//! Packages the installer downloads itself (phase 3, Spec - Sources constructeurs, « Installeur »):
//! look-up at the source, HTTPS download, hash (R-SRC-02), signature (R-SRC-03), opening without
//! running anything (R-SRC-08), INFs read by Windows. Nothing here changes the system: the folders
//! are Drivn's own, under `passes\<pass>\` (admin only).

use std::io::Read;
use std::path::{Path, PathBuf};

use drivn_core::install::actions::{InfFacts, Package, PackageError};
use drivn_core::install::plan::MS_CATALOG;
use drivn_core::install::run::CATALOG_PACKAGE_SIGNERS;
use drivn_core::sources::archive::Limits;
use drivn_core::sources::http::{Request, fetch};
use drivn_core::sources::ms_catalog;
use drivn_windows::authenticode::file_signer;
use drivn_windows::http::WinHttp;
use drivn_windows::inf::read_inf;
use drivn_windows::security::is_reparse_point;

use crate::files::Root;

/// A driver `.cab` of the Catalog is a few MB; above this it is refused.
const MAX_CAB_BYTES: usize = 512 * 1024 * 1024;

pub fn work_dirs(root: &Root, pass_id: &str, n: usize) -> (PathBuf, PathBuf) {
    let pass = root.pass_dir(pass_id);
    (
        pass.join("download").join(n.to_string()),
        pass.join("extract").join(n.to_string()),
    )
}

/// R-SEC-06: a folder found from a cut pass is never reused.
fn fresh(root: &Root, dir: &Path) -> Result<(), String> {
    if std::fs::symlink_metadata(dir).is_ok() {
        if is_reparse_point(dir) {
            return Err(format!("{} is a reparse point", dir.display()));
        }
        std::fs::remove_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    root.ensure(dir)
}

pub fn fetch_catalog(
    root: &Root,
    pass_id: &str,
    n: usize,
    source: &str,
    update_id: &str,
) -> Result<Package, PackageError> {
    if source != MS_CATALOG {
        return Err(PackageError::Download(format!("unknown source {source}")));
    }
    let (download, extract) = work_dirs(root, pass_id, n);
    fresh(root, &download).map_err(PackageError::Download)?;
    fresh(root, &extract).map_err(PackageError::Download)?;

    // R-SEC-05: link and hash asked to the Catalog now, never taken from the plan.
    let found =
        ms_catalog::look_up_download(&WinHttp, update_id).map_err(PackageError::Download)?;
    let bytes = fetch(&WinHttp, Request::get(found.url.clone(), MAX_CAB_BYTES))
        .map_err(|e| PackageError::Download(e.to_string()))?;
    // R-SRC-02: before anything else; a different file is deleted.
    if !ms_catalog::hash_matches(&bytes, &found.sha256) {
        return Err(PackageError::Hash);
    }
    let file = download.join("package.cab");
    std::fs::write(&file, &bytes).map_err(|e| PackageError::Download(e.to_string()))?;
    // R-SRC-03: valid signature, revocation checked online, a Microsoft signer.
    let signer = file_signer(&file, true).map_err(PackageError::Signature)?;
    if !CATALOG_PACKAGE_SIGNERS.contains(&signer.as_str()) {
        let _ = std::fs::remove_file(&file);
        return Err(PackageError::Signature(format!(
            "unexpected signer {signer}"
        )));
    }
    extract_cab(&bytes, &extract).map_err(PackageError::Archive)?;
    let mut infs = Vec::new();
    collect_infs(&extract, &mut infs, 0);
    Ok(Package { infs, signer })
}

/// R-SRC-08: opened by a Rust library, nothing run; names and sizes guarded before each write.
fn extract_cab(bytes: &[u8], target: &Path) -> Result<(), String> {
    let mut cabinet =
        cab::Cabinet::new(std::io::Cursor::new(bytes)).map_err(|e| format!("not a cab: {e}"))?;
    let entries: Vec<(String, u64)> = cabinet
        .folder_entries()
        .flat_map(|folder| {
            folder
                .file_entries()
                .map(|f| (f.name().to_owned(), u64::from(f.uncompressed_size())))
                .collect::<Vec<_>>()
        })
        .collect();
    let compressed = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
    let mut limits = Limits::default();
    for (name, size) in entries {
        let parts = limits.admit(&name, size, compressed)?;
        let mut path = target.to_path_buf();
        for part in &parts {
            path.push(part);
        }
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        let mut reader = cabinet
            .read_file(&name)
            .map_err(|e| format!("{name}: {e}"))?;
        let mut content = Vec::new();
        reader
            .by_ref()
            .take(size.saturating_add(1))
            .read_to_end(&mut content)
            .map_err(|e| format!("{name}: {e}"))?;
        if u64::try_from(content.len()).unwrap_or(u64::MAX) != size {
            return Err(format!("{name}: size differs from the cab's table"));
        }
        std::fs::write(&path, content).map_err(|e| format!("{name}: {e}"))?;
    }
    Ok(())
}

fn collect_infs(dir: &Path, out: &mut Vec<InfFacts>, depth: usize) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if is_reparse_point(&path) {
            continue;
        }
        if path.is_dir() && depth < 8 {
            collect_infs(&path, out, depth + 1);
        } else if path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("inf"))
            && let Ok(facts) = read_inf(&path)
        {
            out.push(facts);
        }
    }
}

/// End of a pass: `download\` and `extract\` are never kept.
pub fn clean(root: &Root, pass_id: &str) -> Result<(), String> {
    let pass = root.pass_dir(pass_id);
    for dir in [pass.join("download"), pass.join("extract")] {
        if std::fs::symlink_metadata(&dir).is_ok() {
            if is_reparse_point(&dir) {
                return Err(format!("{} is a reparse point", dir.display()));
            }
            std::fs::remove_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
    }
    Ok(())
}
