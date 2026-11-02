//! The installer's folders (Spec - Installation, « Fichiers et droits »). `%ProgramData%\Drivn\` is
//! created with an ACL of its own (owner and writers: Administrators, SYSTEM; Users: read) and
//! checked before every write: no reparse point, trusted owner (R-SEC-05). What the app writes,
//! `%ProgramData%\Drivn-requests\`, is only ever read here.

use std::cell::Cell;
use std::fs::OpenOptions;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use drivn_core::install::control::MAX_CONTROL_BYTES;
use drivn_core::install::journal::{Line, to_line};
use drivn_core::install::plan::valid_pass_id;
use drivn_core::install::run::{Journal, PassStore};
use drivn_core::install::setup::{FolderFacts, TRUSTED_OWNERS, check_root};
use drivn_windows::security::{is_reparse_point, owner_sid, program_data};
use windows::Win32::Foundation::{HLOCAL, LocalFree};
use windows::Win32::Security::Authorization::{
    ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1, SE_FILE_OBJECT,
    SetNamedSecurityInfoW,
};
use windows::Win32::Security::{
    ACL, DACL_SECURITY_INFORMATION, GetSecurityDescriptorDacl, PROTECTED_DACL_SECURITY_INFORMATION,
    PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES,
};
use windows::Win32::Storage::FileSystem::CreateDirectoryW;
use windows::core::{BOOL, HSTRING, w};

/// Owner Administrators; full control Administrators and SYSTEM; Users read and list only.
const SDDL: windows::core::PCWSTR =
    w!("O:BAG:SYD:P(A;OICI;FA;;;BA)(A;OICI;FA;;;SY)(A;OICI;0x1200a9;;;BU)");

struct Sd(PSECURITY_DESCRIPTOR);

impl Sd {
    fn new() -> Result<Self, String> {
        let mut sd = PSECURITY_DESCRIPTOR::default();
        // SAFETY: SDDL is a constant string; the descriptor is freed in Drop.
        unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                SDDL,
                SDDL_REVISION_1,
                &mut sd,
                None,
            )
            .map_err(|e| format!("security descriptor: {e}"))?;
        }
        Ok(Self(sd))
    }
}

impl Drop for Sd {
    fn drop(&mut self) {
        // SAFETY: allocated by ConvertStringSecurityDescriptorToSecurityDescriptorW.
        unsafe {
            let _ = LocalFree(Some(HLOCAL(self.0.0)));
        }
    }
}

fn create_protected(dir: &Path) -> Result<(), String> {
    let sd = Sd::new()?;
    let attributes = SECURITY_ATTRIBUTES {
        nLength: u32::try_from(size_of::<SECURITY_ATTRIBUTES>()).unwrap_or(0),
        lpSecurityDescriptor: sd.0.0,
        bInheritHandle: BOOL(0),
    };
    // SAFETY: valid path string and security attributes for the duration of the call.
    unsafe { CreateDirectoryW(&HSTRING::from(dir.as_os_str()), Some(&attributes)) }
        .map_err(|e| format!("{}: {e}", dir.display()))
}

/// Puts Drivn's own ACL back on a folder it owns (another admin tool may have widened it).
fn protect(dir: &Path) -> Result<(), String> {
    let sd = Sd::new()?;
    let mut present = BOOL(0);
    let mut defaulted = BOOL(0);
    let mut dacl: *mut ACL = std::ptr::null_mut();
    // SAFETY: `dacl` points into `sd`, alive until the end of this function.
    unsafe {
        GetSecurityDescriptorDacl(sd.0, &mut present, &mut dacl, &mut defaulted)
            .map_err(|e| e.to_string())?;
        let result = SetNamedSecurityInfoW(
            &HSTRING::from(dir.as_os_str()),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
            None,
            None,
            Some(dacl),
            None,
        );
        if result.is_err() {
            return Err(format!("{}: ACL not set ({})", dir.display(), result.0));
        }
    }
    Ok(())
}

pub struct Root {
    pub drivn: PathBuf,
    pub requests: PathBuf,
    /// Drivn's ACL is put back on the root once per launch, not at every journal line.
    protected: Cell<bool>,
}

impl Root {
    pub fn locate() -> Result<Self, String> {
        let base = program_data()?;
        Ok(Self {
            drivn: base.join("Drivn"),
            requests: base.join("Drivn-requests"),
            protected: Cell::new(false),
        })
    }

    pub fn passes(&self) -> PathBuf {
        self.drivn.join("passes")
    }

    /// `pass_id` must have been checked with `valid_pass_id` before.
    pub fn pass_dir(&self, pass_id: &str) -> PathBuf {
        self.passes().join(pass_id)
    }

    pub fn plan_request(&self, pass_id: &str) -> PathBuf {
        self.requests.join("plans").join(format!("{pass_id}.json"))
    }

    pub fn control(&self, pass_id: &str) -> PathBuf {
        self.requests.join("control").join(format!("{pass_id}.cmd"))
    }

    /// `real-mode`: created by hand by an administrator (R-TST-01, ADR-0020). Read after `ensure`
    /// put Drivn's ACL on the root, and only if an administrator owns it.
    pub fn real_mode(&self) -> bool {
        let file = self.drivn.join("real-mode");
        self.protected.get()
            && file.is_file()
            && !is_reparse_point(&file)
            && owner_sid(&file).is_ok_and(|sid| TRUSTED_OWNERS.contains(&sid.as_str()))
    }

    /// Creates the missing folders from the root down to `dir`, then checks each of them.
    pub fn ensure(&self, dir: &Path) -> Result<(), String> {
        let relative = dir
            .strip_prefix(&self.drivn)
            .map_err(|_| format!("{} is outside of Drivn's folder", dir.display()))?;
        let mut chain = vec![self.drivn.clone()];
        for part in relative.components() {
            let next = chain.last().map(|p| p.join(part)).unwrap_or_default();
            chain.push(next);
        }
        let mut facts = Vec::new();
        for (index, folder) in chain.iter().enumerate() {
            if std::fs::symlink_metadata(folder).is_err() {
                create_protected(folder)?;
            }
            let fact = FolderFacts {
                path: folder.display().to_string(),
                reparse_point: is_reparse_point(folder),
                owner_sid: owner_sid(folder).ok(),
            };
            check_root(std::slice::from_ref(&fact))?;
            if index == 0 && !self.protected.get() {
                protect(folder)?;
                self.protected.set(true);
            }
            facts.push(fact);
        }
        check_root(&facts)
    }

    /// One word of the control file: a plain file of 32 bytes at most, else ignored.
    pub fn read_control(&self, pass_id: &str) -> Option<String> {
        let path = self.control(pass_id);
        let meta = std::fs::symlink_metadata(&path).ok()?;
        if is_reparse_point(&path) || !meta.is_file() || meta.len() > MAX_CONTROL_BYTES {
            return None;
        }
        let mut text = String::new();
        std::fs::File::open(&path)
            .ok()?
            .take(MAX_CONTROL_BYTES)
            .read_to_string(&mut text)
            .ok()?;
        Some(text)
    }
}

/// `journal.jsonl` of one pass: every line reaches the disk before the next step.
pub struct FileJournal {
    root: PathBuf,
    path: PathBuf,
    drivn: Root,
}

impl FileJournal {
    pub fn new(root: &Root, pass_id: &str) -> Self {
        let dir = root.pass_dir(pass_id);
        Self {
            path: dir.join("journal.jsonl"),
            root: dir,
            drivn: Root {
                drivn: root.drivn.clone(),
                requests: root.requests.clone(),
                protected: Cell::new(root.protected.get()),
            },
        }
    }
}

pub fn append_line(path: &Path, line: &Line) -> Result<(), String> {
    if path.exists() && is_reparse_point(path) {
        return Err(format!("{} is a link", path.display()));
    }
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    file.write_all(to_line(line)?.as_bytes())
        .and_then(|()| file.sync_data())
        .map_err(|e| format!("{}: {e}", path.display()))
}

impl Journal for FileJournal {
    fn append(&mut self, line: &Line) -> Result<(), String> {
        self.drivn.ensure(&self.root)?;
        append_line(&self.path, line)
    }
}

pub struct Passes<'a>(pub &'a Root);

impl PassStore for Passes<'_> {
    fn passes(&mut self) -> Vec<(String, String)> {
        let Ok(entries) = std::fs::read_dir(self.0.passes()) else {
            return Vec::new();
        };
        entries
            .filter_map(Result::ok)
            .filter_map(|entry| {
                let id = entry.file_name().to_string_lossy().into_owned();
                if !valid_pass_id(&id) || is_reparse_point(&entry.path()) {
                    return None;
                }
                let text = std::fs::read_to_string(entry.path().join("journal.jsonl")).ok()?;
                Some((id, text))
            })
            .collect()
    }

    fn append(&mut self, pass_id: &str, line: &Line) -> Result<(), String> {
        if !valid_pass_id(pass_id) {
            return Err("invalid pass id".to_owned());
        }
        let dir = self.0.pass_dir(pass_id);
        self.0.ensure(&dir)?;
        append_line(&dir.join("journal.jsonl"), line)
    }
}
