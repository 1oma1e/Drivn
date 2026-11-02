//! R-SRC-11: few requests to a fragile source, and none for 24 h once it has refused Drivn.
//! The state (blocks and answers kept 24 h) is saved by the app in `%LocalAppData%\Drivn\cache\`
//! so a block survives a restart; the count of requests is per scan.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::SourceId;

pub const MAX_REQUESTS_PER_SCAN: u32 = 60;
pub const BLOCK_SECONDS: i64 = 24 * 3600;
pub const CACHE_SECONDS: i64 = 24 * 3600;
/// ADR-0031: an unreadable page is asked again once, after this pause.
pub const RETRY_SECONDS: u64 = 5;
/// ADR-0031: the refused page is kept for analysis, cut to this size.
pub const MAX_REJECTED_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GuardState {
    /// Source key → Unix time until which it is not called.
    #[serde(default)]
    pub blocked_until: BTreeMap<String, i64>,
    /// Why it was blocked, shown in the details.
    #[serde(default)]
    pub blocked_reason: BTreeMap<String, String>,
    /// The page that made the source blocked (ADR-0031), kept while the block lasts.
    #[serde(default)]
    pub rejected_page: BTreeMap<String, String>,
    /// `source|request key` → answer kept 24 h.
    #[serde(default)]
    pub cache: BTreeMap<String, Cached>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cached {
    pub at: i64,
    pub body: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Permit {
    /// The answer of less than 24 h; no request is made.
    Cached(String),
    Go,
    Blocked {
        until: i64,
        reason: String,
    },
    LimitReached,
}

#[derive(Debug, Clone, Default)]
pub struct Guard {
    pub state: GuardState,
    counts: BTreeMap<SourceId, u32>,
}

impl Guard {
    pub fn new(state: GuardState, now: i64) -> Self {
        let mut guard = Self {
            state,
            counts: BTreeMap::new(),
        };
        guard.prune(now);
        guard
    }

    fn cache_key(source: SourceId, key: &str) -> String {
        format!("{}|{key}", source.key())
    }

    /// Drops blocks and answers older than 24 h.
    pub fn prune(&mut self, now: i64) {
        self.state.blocked_until.retain(|_, until| *until > now);
        let blocked = self.state.blocked_until.clone();
        self.state
            .blocked_reason
            .retain(|source, _| blocked.contains_key(source));
        self.state
            .rejected_page
            .retain(|source, _| blocked.contains_key(source));
        self.state
            .cache
            .retain(|_, c| now - c.at < CACHE_SECONDS && c.at <= now);
    }

    pub fn blocked(&self, source: SourceId, now: i64) -> Option<(i64, String)> {
        let until = *self.state.blocked_until.get(source.key())?;
        (until > now).then(|| {
            let reason = self
                .state
                .blocked_reason
                .get(source.key())
                .cloned()
                .unwrap_or_default();
            (until, reason)
        })
    }

    /// Asks before each request. `Go` counts the request.
    pub fn permit(&mut self, source: SourceId, key: &str, now: i64) -> Permit {
        if let Some((until, reason)) = self.blocked(source, now) {
            return Permit::Blocked { until, reason };
        }
        if let Some(cached) = self.state.cache.get(&Self::cache_key(source, key))
            && now - cached.at < CACHE_SECONDS
            && cached.at <= now
        {
            return Permit::Cached(cached.body.clone());
        }
        let count = self.counts.entry(source).or_insert(0);
        if *count >= MAX_REQUESTS_PER_SCAN {
            return Permit::LimitReached;
        }
        *count += 1;
        Permit::Go
    }

    pub fn requests(&self, source: SourceId) -> u32 {
        self.counts.get(&source).copied().unwrap_or(0)
    }

    pub fn keep(&mut self, source: SourceId, key: &str, now: i64, body: String) {
        self.state
            .cache
            .insert(Self::cache_key(source, key), Cached { at: now, body });
    }

    /// 403, 429, CAPTCHA or unexpected format: no request to this source for 24 h.
    pub fn block(&mut self, source: SourceId, now: i64, reason: &str) {
        self.state
            .blocked_until
            .insert(source.key().to_owned(), now + BLOCK_SECONDS);
        self.state
            .blocked_reason
            .insert(source.key().to_owned(), reason.to_owned());
    }

    /// Keeps the page that made the source blocked (ADR-0031); it never leaves the PC.
    pub fn keep_rejected(&mut self, source: SourceId, page: &str) {
        let mut end = page.len().min(MAX_REJECTED_BYTES);
        while !page.is_char_boundary(end) {
            end -= 1;
        }
        self.state
            .rejected_page
            .insert(source.key().to_owned(), page[..end].to_owned());
    }

    /// Forgets a kept answer that turned out unreadable.
    pub fn forget(&mut self, source: SourceId, key: &str) {
        self.state.cache.remove(&Self::cache_key(source, key));
    }
}
