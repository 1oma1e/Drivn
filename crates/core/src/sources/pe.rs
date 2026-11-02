//! R-SRC-08 for an executable: the Authenticode signature covers the file up to its certificate
//! table, which must be the very end of the file. Bytes added after it are not covered by the
//! signature and could hide another payload behind a valid one: such a file is refused.

/// Bytes to read at the start of the file to find the certificate table.
pub const HEADER_BYTES: usize = 4096;

fn u16_at(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(at..at + 2)?.try_into().ok()?))
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

/// `(offset, size)` of the certificate table (security directory) of a PE file.
pub fn certificate_table(header: &[u8]) -> Result<(u64, u64), String> {
    if header.get(..2) != Some(b"MZ") {
        return Err("not an executable".to_owned());
    }
    let pe = usize::try_from(u32_at(header, 0x3c).ok_or("truncated header")?)
        .map_err(|_| "bad header")?;
    if header.get(pe..pe + 4) != Some(b"PE\0\0") {
        return Err("no PE signature".to_owned());
    }
    let optional = pe + 24;
    let directories = match u16_at(header, optional).ok_or("truncated header")? {
        0x10b => optional + 96,  // PE32
        0x20b => optional + 112, // PE32+
        _ => return Err("unknown optional header".to_owned()),
    };
    // Entry 4 of the data directories: the certificate table (a file offset, not an RVA).
    let entry = directories + 4 * 8;
    let offset = u32_at(header, entry).ok_or("truncated header")?;
    let size = u32_at(header, entry + 4).ok_or("truncated header")?;
    if offset == 0 || size == 0 {
        return Err("not signed".to_owned());
    }
    Ok((u64::from(offset), u64::from(size)))
}

/// The certificate table ends the file (up to 7 bytes of padding to 8, which Windows allows).
pub fn nothing_after_signature(header: &[u8], file_len: u64) -> Result<(), String> {
    let (offset, size) = certificate_table(header)?;
    let end = offset.checked_add(size).ok_or("bad certificate table")?;
    if end > file_len {
        return Err("certificate table beyond the end of the file".to_owned());
    }
    if file_len - end > 7 {
        return Err(format!(
            "{} bytes after the signed zone: refused",
            file_len - end
        ));
    }
    Ok(())
}
