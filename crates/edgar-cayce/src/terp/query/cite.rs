//! Citation wrapper for one LCD read.
//!
//! Footers follow design §3.6: chain id, host only, path, height, UTC time.
//! Cached reads add `cached Ns ago`.

use chrono::{DateTime, Utc};

/// One typed LCD result plus where it came from.
#[derive(Debug, Clone)]
pub struct Cited<T> {
    pub data: T,
    pub height: i64,
    /// Host only (`192.168.10.101:1317`), never the scheme or path.
    pub endpoint: String,
    pub path: String,
    pub fetched_at: DateTime<Utc>,
    pub cached: bool,
}

impl<T> Cited<T> {
    pub fn map<U>(self, f: impl FnOnce(T) -> U) -> Cited<U> {
        Cited {
            data: f(self.data),
            height: self.height,
            endpoint: self.endpoint,
            path: self.path,
            fetched_at: self.fetched_at,
            cached: self.cached,
        }
    }

    pub fn source(&self) -> Source {
        Source::from_cited(self)
    }
}

/// The same provenance without the payload. Tally rides along on a proposal this way.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Source {
    pub height: i64,
    pub endpoint: String,
    pub path: String,
    pub fetched_at: DateTime<Utc>,
    pub cached: bool,
}

impl Source {
    pub fn from_cited<T>(c: &Cited<T>) -> Self {
        Self {
            height: c.height,
            endpoint: c.endpoint.clone(),
            path: c.path.clone(),
            fetched_at: c.fetched_at,
            cached: c.cached,
        }
    }

    pub fn line(&self) -> String {
        citation_parts(
            &self.endpoint,
            &self.path,
            self.height,
            self.fetched_at,
            self.cached,
        )
    }
}

pub fn citation<T>(c: &Cited<T>) -> String {
    citation_parts(&c.endpoint, &c.path, c.height, c.fetched_at, c.cached)
}

pub fn citation_parts(
    endpoint: &str,
    path: &str,
    height: i64,
    fetched_at: DateTime<Utc>,
    cached: bool,
) -> String {
    let when = fetched_at.format("%Y-%m-%d %H:%M GMT");
    let mut line =
        format!("— source: morocco-1 LCD {endpoint} · {path} · height {height} · {when}");
    if cached {
        let age = (Utc::now() - fetched_at).num_seconds().max(0);
        line.push_str(&format!(" · cached {age}s ago"));
    }
    line
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn footer_matches_the_design_shape() {
        let fetched_at = Utc.with_ymd_and_hms(2026, 10, 16, 3, 10, 4).unwrap();
        let cited = Cited {
            data: (),
            height: 1_234_567,
            endpoint: "terp.api.nodex.one".into(),
            path: "/cosmos/gov/v1/proposals/62".into(),
            fetched_at,
            cached: false,
        };
        assert_eq!(
            citation(&cited),
            "— source: morocco-1 LCD terp.api.nodex.one · /cosmos/gov/v1/proposals/62 · height 1234567 · 2026-10-16 03:10 GMT"
        );
    }

    #[test]
    fn cached_footer_names_the_age() {
        let fetched_at = Utc::now() - chrono::Duration::seconds(20);
        let line = citation_parts("192.168.10.101:1317", "/x", 9, fetched_at, true);
        let age = (Utc::now() - fetched_at).num_seconds();
        assert!(line.contains(&format!("cached {age}s ago")));
        assert!(line.contains("192.168.10.101:1317"));
    }
}
