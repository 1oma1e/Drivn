//! What Drivn sends to a source, built here so R-SRC-01 and R-SRC-11 are tested on it: HTTPS to
//! an allowed host only, Drivn's own `User-Agent`, no `Referer`, no cookie, redirects followed
//! only to an allowed host. The real client (WinHTTP) lives in `drivn-windows`.

use super::hosts::{HttpsUrl, UrlRefused};

pub const USER_AGENT: &str = concat!("Drivn/", env!("CARGO_PKG_VERSION"));
/// R-SRC-11: a page or JSON answer read from a site is at most 2 MB.
pub const MAX_PAGE_BYTES: usize = 2 * 1024 * 1024;
const MAX_REDIRECTS: usize = 3;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub url: HttpsUrl,
    /// `application/x-www-form-urlencoded` body of a POST; `None` for a GET.
    pub form: Option<String>,
    /// The only headers sent. The client adds nothing else (no cookie store, no `Referer`).
    pub headers: Vec<(&'static str, String)>,
    pub max_bytes: usize,
}

impl Request {
    pub fn get(url: HttpsUrl, max_bytes: usize) -> Self {
        Self {
            url,
            form: None,
            headers: vec![("User-Agent", USER_AGENT.to_owned())],
            max_bytes,
        }
    }

    pub fn post_form(url: HttpsUrl, form: String, max_bytes: usize) -> Self {
        let mut request = Self::get(url, max_bytes);
        request.headers.push((
            "Content-Type",
            "application/x-www-form-urlencoded".to_owned(),
        ));
        request.form = Some(form);
        request
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Response {
    pub status: u16,
    /// `Location` of a 3xx answer. The client never follows a redirect by itself.
    pub location: Option<String>,
    pub body: Vec<u8>,
}

/// One request, one answer. A body over `max_bytes` is an error, never cut silently.
pub trait Http {
    fn send(&self, request: &Request) -> Result<Response, String>;

    /// Waits before asking again (ADR-0031). Fakes of the tests do not wait.
    fn pause(&self, _seconds: u64) {}
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FetchError {
    Refused(UrlRefused),
    /// 403, 429: the source refuses Drivn (R-SRC-11).
    Blocked(u16),
    Status(u16),
    Network(String),
}

impl std::fmt::Display for FetchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Refused(why) => write!(f, "refused: {why}"),
            Self::Blocked(code) => write!(f, "the site refused Drivn (HTTP {code})"),
            Self::Status(code) => write!(f, "HTTP {code}"),
            Self::Network(e) => write!(f, "network: {e}"),
        }
    }
}

/// Sends `request`, following at most 3 redirects, each to an allowed host (R-SRC-01).
/// A redirected POST becomes a GET without body, as browsers do for 301, 302 and 303.
pub fn fetch(http: &dyn Http, request: Request) -> Result<Vec<u8>, FetchError> {
    let mut request = request;
    for _ in 0..=MAX_REDIRECTS {
        let response = http.send(&request).map_err(FetchError::Network)?;
        match response.status {
            200 => return Ok(response.body),
            301 | 302 | 303 | 307 | 308 => {
                let location = response
                    .location
                    .ok_or(FetchError::Status(response.status))?;
                let next = request.url.join(&location).map_err(FetchError::Refused)?;
                request = if matches!(response.status, 307 | 308) {
                    Request {
                        url: next,
                        ..request
                    }
                } else {
                    Request::get(next, request.max_bytes)
                };
            }
            403 | 429 => return Err(FetchError::Blocked(response.status)),
            other => return Err(FetchError::Status(other)),
        }
    }
    Err(FetchError::Status(310))
}
