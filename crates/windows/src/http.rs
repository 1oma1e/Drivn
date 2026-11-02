//! HTTPS client of Drivn: WinHTTP (Windows' own TLS, certificates and proxy, no new TLS library).
//! It sends only the headers of the [`Request`] built in `drivn-core` (R-SRC-11), keeps no cookie
//! and never follows a redirect by itself: `fetch` checks each one against the host list (R-SRC-01).

use std::ffi::c_void;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use std::io::Write;
use std::path::Path;

use drivn_core::sources::hosts::HttpsUrl;
use drivn_core::sources::http::{Http, Request, Response};
use windows::Win32::Networking::WinHttp::{
    INTERNET_DEFAULT_HTTPS_PORT, WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, WINHTTP_ADDREQ_FLAG_ADD,
    WINHTTP_DISABLE_COOKIES, WINHTTP_FLAG_SECURE, WINHTTP_OPTION_DISABLE_FEATURE,
    WINHTTP_OPTION_REDIRECT_POLICY, WINHTTP_OPTION_REDIRECT_POLICY_NEVER,
    WINHTTP_QUERY_FLAG_NUMBER, WINHTTP_QUERY_LOCATION, WINHTTP_QUERY_STATUS_CODE,
    WinHttpAddRequestHeaders, WinHttpCloseHandle, WinHttpConnect, WinHttpOpen, WinHttpOpenRequest,
    WinHttpQueryHeaders, WinHttpReadData, WinHttpReceiveResponse, WinHttpSendRequest,
    WinHttpSetOption, WinHttpSetTimeouts,
};
use windows::core::{HSTRING, PCWSTR};

pub struct WinHttp;

/// Closes a WinHTTP handle when dropped.
struct Handle(*mut c_void);

impl Drop for Handle {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: a handle returned by WinHTTP, closed once.
            let _ = unsafe { WinHttpCloseHandle(self.0) };
        }
    }
}

fn last_error(what: &str) -> String {
    format!("{what}: {}", windows::core::Error::from_thread())
}

fn set_u32(handle: &Handle, option: u32, value: u32) -> Result<(), String> {
    let bytes = value.to_ne_bytes();
    // SAFETY: the option takes a DWORD; the buffer lives for the call.
    unsafe { WinHttpSetOption(Some(handle.0), option, Some(&bytes)) }
        .map_err(|e| format!("WinHttpSetOption: {e}"))
}

/// R-SRC-11: at least this long between two requests of Drivn, whatever the source. Seen on
/// 2026-11-01: the Catalog answers an unreadable page to requests sent back to back.
const MIN_INTERVAL: Duration = Duration::from_secs(1);
static LAST_REQUEST: Mutex<Option<Instant>> = Mutex::new(None);

fn pace() {
    let Ok(mut last) = LAST_REQUEST.lock() else {
        std::thread::sleep(MIN_INTERVAL);
        return;
    };
    if let Some(at) = *last {
        let since = at.elapsed();
        if since < MIN_INTERVAL {
            std::thread::sleep(MIN_INTERVAL - since);
        }
    }
    *last = Some(Instant::now());
}

/// One request; the body goes to `sink` chunk by chunk. Returns the status and `Location`.
fn exchange(
    request: &Request,
    sink: &mut dyn FnMut(&[u8]) -> Result<(), String>,
) -> Result<(u16, Option<String>), String> {
    pace();
    let agent = request
        .headers
        .iter()
        .find(|(name, _)| *name == "User-Agent")
        .map(|(_, v)| v.as_str())
        .unwrap_or_default();
    // SAFETY: plain WinHTTP calls; every string outlives its call, every handle is closed.
    unsafe {
        let session = Handle(WinHttpOpen(
            &HSTRING::from(agent),
            WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
            PCWSTR::null(),
            PCWSTR::null(),
            0,
        ));
        if session.0.is_null() {
            return Err(last_error("WinHttpOpen"));
        }
        WinHttpSetTimeouts(session.0, 15_000, 15_000, 30_000, 60_000)
            .map_err(|e| format!("WinHttpSetTimeouts: {e}"))?;
        let connect = Handle(WinHttpConnect(
            session.0,
            &HSTRING::from(request.url.host()),
            INTERNET_DEFAULT_HTTPS_PORT,
            0,
        ));
        if connect.0.is_null() {
            return Err(last_error("WinHttpConnect"));
        }
        let verb = if request.form.is_some() {
            "POST"
        } else {
            "GET"
        };
        let handle = Handle(WinHttpOpenRequest(
            connect.0,
            &HSTRING::from(verb),
            &HSTRING::from(request.url.path()),
            PCWSTR::null(),
            PCWSTR::null(),
            std::ptr::null(),
            WINHTTP_FLAG_SECURE,
        ));
        if handle.0.is_null() {
            return Err(last_error("WinHttpOpenRequest"));
        }
        set_u32(
            &handle,
            WINHTTP_OPTION_REDIRECT_POLICY,
            WINHTTP_OPTION_REDIRECT_POLICY_NEVER,
        )?;
        set_u32(
            &handle,
            WINHTTP_OPTION_DISABLE_FEATURE,
            WINHTTP_DISABLE_COOKIES,
        )?;
        for (name, value) in request.headers.iter().filter(|(n, _)| *n != "User-Agent") {
            let line: Vec<u16> = format!("{name}: {value}").encode_utf16().collect();
            WinHttpAddRequestHeaders(handle.0, &line, WINHTTP_ADDREQ_FLAG_ADD)
                .map_err(|e| format!("WinHttpAddRequestHeaders: {e}"))?;
        }
        let body = request.form.as_deref().unwrap_or_default().as_bytes();
        let length = u32::try_from(body.len()).map_err(|_| "body too large".to_owned())?;
        WinHttpSendRequest(
            handle.0,
            None,
            (!body.is_empty()).then_some(body.as_ptr().cast()),
            length,
            length,
            0,
        )
        .map_err(|e| format!("WinHttpSendRequest: {e}"))?;
        WinHttpReceiveResponse(handle.0, std::ptr::null_mut())
            .map_err(|e| format!("WinHttpReceiveResponse: {e}"))?;

        let mut status: u32 = 0;
        let mut size = u32::try_from(size_of::<u32>()).unwrap_or(4);
        WinHttpQueryHeaders(
            handle.0,
            WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
            PCWSTR::null(),
            Some(std::ptr::from_mut(&mut status).cast()),
            &mut size,
            std::ptr::null_mut(),
        )
        .map_err(|e| format!("status: {e}"))?;
        let mut location = None;
        if (300..400).contains(&status) {
            let mut buffer = [0u16; 2048];
            let mut bytes = u32::try_from(size_of_val(&buffer)).unwrap_or(0);
            if WinHttpQueryHeaders(
                handle.0,
                WINHTTP_QUERY_LOCATION,
                PCWSTR::null(),
                Some(buffer.as_mut_ptr().cast()),
                &mut bytes,
                std::ptr::null_mut(),
            )
            .is_ok()
            {
                let chars = usize::try_from(bytes).unwrap_or(0) / 2;
                location = Some(String::from_utf16_lossy(&buffer[..chars.min(buffer.len())]));
            }
            return Ok((u16::try_from(status).unwrap_or(0), location));
        }

        let mut chunk = vec![0u8; 256 * 1024];
        loop {
            let mut read: u32 = 0;
            WinHttpReadData(
                handle.0,
                chunk.as_mut_ptr().cast(),
                u32::try_from(chunk.len()).unwrap_or(0),
                &mut read,
            )
            .map_err(|e| format!("WinHttpReadData: {e}"))?;
            if read == 0 {
                break;
            }
            sink(&chunk[..usize::try_from(read).unwrap_or(0)])?;
        }
        Ok((u16::try_from(status).unwrap_or(0), location))
    }
}

impl Http for WinHttp {
    fn send(&self, request: &Request) -> Result<Response, String> {
        let mut out = Vec::new();
        let (status, location) = exchange(request, &mut |bytes| {
            if out.len() + bytes.len() > request.max_bytes {
                return Err(format!("answer larger than {} bytes", request.max_bytes));
            }
            out.extend_from_slice(bytes);
            Ok(())
        })?;
        Ok(Response {
            status,
            location,
            body: out,
        })
    }

    fn pause(&self, seconds: u64) {
        std::thread::sleep(Duration::from_secs(seconds));
    }
}

/// A large file (a publisher's package) written straight to `path`, never held in memory.
/// Redirects are followed only to an allowed host (R-SRC-01), 3 at most. `progress(written)`.
pub fn download_to_file(
    url: &HttpsUrl,
    path: &Path,
    max_bytes: u64,
    progress: &mut dyn FnMut(u64),
) -> Result<u64, String> {
    let mut url = url.clone();
    for _ in 0..=3 {
        let mut file =
            std::fs::File::create(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let mut written: u64 = 0;
        let request = Request::get(url.clone(), 0);
        let (status, location) = exchange(&request, &mut |bytes| {
            written += u64::try_from(bytes.len()).unwrap_or(u64::MAX);
            if written > max_bytes {
                return Err(format!("file larger than {max_bytes} bytes"));
            }
            file.write_all(bytes).map_err(|e| e.to_string())?;
            progress(written);
            Ok(())
        })?;
        match status {
            200 => {
                file.sync_all().map_err(|e| e.to_string())?;
                return Ok(written);
            }
            301 | 302 | 303 | 307 | 308 => {
                let next = location.ok_or("redirect without a location")?;
                url = url.join(&next).map_err(|e| e.to_string())?;
            }
            other => return Err(format!("HTTP {other}")),
        }
    }
    Err("too many redirects".to_owned())
}
