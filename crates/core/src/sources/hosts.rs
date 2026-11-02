//! R-SRC-01: the only hosts Drivn may call, a copy of the "Hôtes de téléchargement" section of
//! docs/SOURCES.md (`tools/check-docs.ps1` checks every `https://` host of the code against it).

/// Written with their scheme so `check-docs` sees them.
const DOWNLOAD_HOSTS: &[&str] = &[
    "https://dl.dell.com",
    "https://downloads.dell.com",
    "https://www.catalog.update.microsoft.com",
    "https://catalog.s.download.windowsupdate.com",
    "https://gfwsl.geforce.com",
    "https://us.download.nvidia.com",
    "https://javadl-esd-secure.oracle.com",
    "https://rog.asus.com",
    "https://www.intel.com",
    "https://downloadmirror.intel.com",
];

/// An HTTPS URL on an allowed host, nothing else: no user, no port, no fragment, no space.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpsUrl {
    host: String,
    /// Starts with `/`.
    path: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UrlRefused {
    NotHttps,
    Malformed,
    HostNotAllowed(String),
}

impl std::fmt::Display for UrlRefused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotHttps => write!(f, "not an HTTPS URL"),
            Self::Malformed => write!(f, "malformed URL"),
            Self::HostNotAllowed(host) => write!(f, "host {host} is not a download host"),
        }
    }
}

pub fn allowed_host(host: &str) -> bool {
    DOWNLOAD_HOSTS
        .iter()
        .any(|h| h.strip_prefix("https://") == Some(host))
}

impl HttpsUrl {
    pub fn parse(url: &str) -> Result<Self, UrlRefused> {
        if url.bytes().any(|b| b <= b' ' || b == 0x7f || b == b'\\') {
            return Err(UrlRefused::Malformed);
        }
        let rest = match url.get(..8) {
            Some(scheme) if scheme.eq_ignore_ascii_case("https://") => &url[8..],
            _ => return Err(UrlRefused::NotHttps),
        };
        let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
        let (host, path) = rest.split_at(end);
        if host.is_empty() || host.contains(['@', ':']) || path.contains('#') {
            return Err(UrlRefused::Malformed);
        }
        let host = host.to_ascii_lowercase();
        if !allowed_host(&host) {
            return Err(UrlRefused::HostNotAllowed(host));
        }
        let path = if path.starts_with('/') {
            path.to_owned()
        } else {
            format!("/{path}")
        };
        Ok(Self { host, path })
    }

    pub fn host(&self) -> &str {
        &self.host
    }

    /// Path and query, as sent on the request line.
    pub fn path(&self) -> &str {
        &self.path
    }

    /// A relative redirect (`/x`) stays on this host; an absolute one is checked like any URL.
    pub fn join(&self, location: &str) -> Result<Self, UrlRefused> {
        if location.starts_with('/') && !location.starts_with("//") {
            Self::parse(&format!("https://{}{location}", self.host))
        } else {
            Self::parse(location)
        }
    }
}

impl std::fmt::Display for HttpsUrl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "https://{}{}", self.host, self.path)
    }
}

/// Percent-encodes a value put in a query string (an ID with `&` stays one value).
pub fn encode_query(value: &str) -> String {
    let mut out = String::new();
    for b in value.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}
