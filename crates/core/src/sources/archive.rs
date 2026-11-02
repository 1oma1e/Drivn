//! R-SRC-08: guards for a package opened as an archive, without running anything (Spec - Sources
//! constructeurs, section 2, step 3). Pure: the reader of the archive asks before each file.

pub const MAX_FILES: usize = 20_000;
pub const MAX_TOTAL_BYTES: u64 = 4 * 1024 * 1024 * 1024;
/// Uncompressed size / compressed size, for the whole archive.
pub const MAX_RATIO: u64 = 100;

const RESERVED: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// The parts of a name that stays under the target folder, or why it is refused.
pub fn safe_parts(name: &str) -> Result<Vec<String>, String> {
    let refuse = |why: &str| Err(format!("{name}: {why}"));
    if name.is_empty() || name.contains('\0') {
        return refuse("empty name");
    }
    if name.starts_with(['\\', '/']) || name.starts_with("\\\\?\\") {
        return refuse("absolute path");
    }
    if name.as_bytes().get(1) == Some(&b':') {
        return refuse("drive letter");
    }
    if name.contains(':') {
        return refuse("alternate data stream");
    }
    let mut parts = Vec::new();
    for part in name.split(['\\', '/']) {
        if part.is_empty() || part == "." {
            continue;
        }
        if part == ".." {
            return refuse("goes up");
        }
        if part.ends_with(['.', ' ']) {
            return refuse("ends with a dot or a space");
        }
        if part
            .chars()
            .any(|c| c.is_control() || "<>\"|?*".contains(c))
        {
            return refuse("forbidden character");
        }
        let stem = part.split('.').next().unwrap_or(part).to_ascii_uppercase();
        if RESERVED.contains(&stem.trim_end()) {
            return refuse("reserved name");
        }
        parts.push(part.to_owned());
    }
    if parts.is_empty() {
        return refuse("empty name");
    }
    Ok(parts)
}

/// Counts what is extracted and refuses past the limits.
#[derive(Debug, Default)]
pub struct Limits {
    files: usize,
    total: u64,
    names: std::collections::HashSet<String>,
}

impl Limits {
    /// Before writing one file of `size` bytes named `name`.
    pub fn admit(
        &mut self,
        name: &str,
        size: u64,
        compressed_total: u64,
    ) -> Result<Vec<String>, String> {
        let parts = safe_parts(name)?;
        let key = parts.join("\\").to_lowercase();
        if !self.names.insert(key) {
            return Err(format!("{name}: two names equal when case is ignored"));
        }
        self.files += 1;
        self.total = self.total.saturating_add(size);
        if self.files > MAX_FILES {
            return Err(format!("more than {MAX_FILES} files"));
        }
        if self.total > MAX_TOTAL_BYTES {
            return Err("more than 4 GB once extracted".to_owned());
        }
        if self.total > compressed_total.max(1).saturating_mul(MAX_RATIO) {
            return Err(format!("compression ratio above {MAX_RATIO}"));
        }
        Ok(parts)
    }
}
