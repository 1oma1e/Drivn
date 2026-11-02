//! Text read on a site is only text (R-VRAI-01): tags removed, entities decoded, cut to 200
//! characters. The interface still shows it with `textContent`, never as HTML.

pub const MAX_CHARS: usize = 200;

/// Removes tags, decodes the common entities, collapses spaces, cuts to [`MAX_CHARS`].
pub fn plain(html: &str) -> String {
    let mut text = String::new();
    let mut in_tag = false;
    for c in html.chars() {
        match c {
            '<' => in_tag = true,
            '>' if in_tag => {
                in_tag = false;
                text.push(' ');
            }
            _ if !in_tag => text.push(c),
            _ => {}
        }
    }
    let decoded = decode_entities(&text);
    let collapsed: String = decoded
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .filter(|c| !c.is_control())
        .collect();
    collapsed.chars().take(MAX_CHARS).collect()
}

fn decode_entities(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(start) = rest.find('&') {
        out.push_str(&rest[..start]);
        let tail = &rest[start..];
        let end = tail.find(';').filter(|e| *e <= 10);
        let decoded = end.and_then(|e| match &tail[1..e] {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" | "#39" => Some('\''),
            "nbsp" => Some(' '),
            _ => None,
        });
        match (end, decoded) {
            (Some(e), Some(c)) => {
                out.push(c);
                rest = &tail[e + 1..];
            }
            _ => {
                out.push('&');
                rest = &tail[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// Decodes `%XX` (NVIDIA writes its texts this way). An invalid escape is kept as it is.
pub fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = |b: u8| char::from(b).to_digit(16);
        if bytes[i] == b'%'
            && let (Some(Some(h)), Some(Some(l))) = (
                bytes.get(i + 1).map(|b| hex(*b)),
                bytes.get(i + 2).map(|b| hex(*b)),
            )
        {
            out.push(u8::try_from(h * 16 + l).unwrap_or(b'?'));
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Standard base64 (with `=` padding), as the Catalog gives its hashes.
pub fn base64_decode(text: &str) -> Option<Vec<u8>> {
    let value = |c: u8| match c {
        b'A'..=b'Z' => Some(c - b'A'),
        b'a'..=b'z' => Some(c - b'a' + 26),
        b'0'..=b'9' => Some(c - b'0' + 52),
        b'+' => Some(62),
        b'/' => Some(63),
        _ => None,
    };
    let trimmed = text.trim().trim_end_matches('=');
    let mut out = Vec::new();
    let mut acc: u32 = 0;
    let mut bits = 0;
    for c in trimmed.bytes() {
        acc = (acc << 6) | u32::from(value(c)?);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(u8::try_from((acc >> bits) & 0xff).ok()?);
        }
    }
    Some(out)
}

/// Lowercase hexadecimal, two digits per byte (hashes).
pub fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(char::from(DIGITS[usize::from(b >> 4)]));
        out.push(char::from(DIGITS[usize::from(b & 0x0f)]));
    }
    out
}

/// Order of the three numbers of a date written with slashes.
#[derive(Clone, Copy)]
pub enum DateOrder {
    /// `2026/07/02`
    Ymd,
    /// `07/02/2026`, as US sites and firmware write it.
    Mdy,
}

/// `YYYY-MM-DD` from a date with slashes and two-digit month and day; anything after the date
/// (a time) is ignored. `None` if it is not exactly such a date.
pub fn slash_date(text: &str, order: DateOrder) -> Option<String> {
    let day = text.split_whitespace().next()?;
    let mut parts = day.split('/');
    let (a, b, c) = (parts.next()?, parts.next()?, parts.next()?);
    let (y, m, d) = match order {
        DateOrder::Ymd => (a, b, c),
        DateOrder::Mdy => (c, a, b),
    };
    let ok = |s: &str, n| s.len() == n && s.bytes().all(|b| b.is_ascii_digit());
    (ok(y, 4) && ok(m, 2) && ok(d, 2) && parts.next().is_none()).then(|| format!("{y}-{m}-{d}"))
}
