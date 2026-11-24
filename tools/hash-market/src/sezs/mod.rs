//! SEZS capability surface on the hash-market server.
//!
//! Product implementations live in permissionlessweb/sezs (`crates/sezs-server`).
//! This module defines the traits those implementations satisfy, a path alias
//! so `/itl/*` still reaches `/sezs/*`, and a nest helper for `AppState`.

#[cfg(feature = "server")]
use axum::Router;
#[cfg(feature = "server")]
use std::sync::Arc;

/// Canonicalise a leftover `/itl/...` request path to `/sezs/...`.
/// Paths that already start with `/sezs` are returned unchanged.
pub fn canonical_path(path: &str) -> String {
    let path = if let Some(rest) = path.strip_prefix("/itl/") {
        format!("/sezs/{rest}")
    } else if path == "/itl" {
        "/sezs".to_string()
    } else {
        path.to_string()
    };
    // After `Router::nest("/sezs", ...)`, some axum versions expose the inner
    // path (`/frost/jobs`) to handlers. Product handlers still key on `/sezs/*`.
    if path == "/session"
        || path.starts_with("/session/")
        || path.starts_with("/frost/")
        || path.starts_with("/door/")
    {
        format!("/sezs{path}")
    } else {
        path
    }
}

/// Top-level SEZS app mounted at `/sezs` (and `/itl` as a short alias).
#[cfg(feature = "server")]
pub trait SezsService: Send + Sync + 'static {
    fn routes(&self) -> Router;
}

/// Invoice, payment watch, leaf/root publish, cancel/refund (sezs-0010).
#[cfg(feature = "server")]
pub trait TicketingProcess: Send + Sync {
    fn routes(&self) -> Router;
}

/// Passkey / ZID session (sezs-0010).
#[cfg(feature = "server")]
pub trait SessionProcess: Send + Sync {
    fn routes(&self) -> Router;
}

/// Email challenge and provider adapter (sezs-0012).
#[cfg(feature = "server")]
pub trait EmailProcess: Send + Sync {
    fn routes(&self) -> Router;
}

/// Formula API client/target for the Argus-fork indexer (sezs-0011).
/// The fork itself is a separate workstream; this trait only describes the API.
pub trait IndexerProcess: Send + Sync {
    /// `GET {base}/{chain_id}/contract/{address}/{formula}?{args}`
    fn formula_url(
        &self,
        base: &str,
        chain_id: &str,
        address: &str,
        formula: &str,
        args: &[(&str, &str)],
    ) -> String {
        let mut url = format!(
            "{}/{}/contract/{}/{}",
            base.trim_end_matches('/'),
            chain_id,
            address,
            formula
        );
        if !args.is_empty() {
            let q = args
                .iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect::<Vec<_>>()
                .join("&");
            url.push('?');
            url.push_str(&q);
        }
        url
    }
}

/// FROST payroll inbox (sezs-0022). Built on the renamed `sezs_frost` module.
#[cfg(feature = "server")]
pub trait PayrollProcess: Send + Sync {
    fn routes(&self) -> Router;
}

/// Nest a SEZS implementation at `/sezs`, with `/itl` as a compatibility alias.
#[cfg(feature = "server")]
pub fn nest_sezs(router: Router, svc: Arc<dyn SezsService>) -> Router {
    router
        .nest("/sezs", svc.routes())
        .nest("/itl", svc.routes())
}

/// Built-in FROST inbox, door, and session routes that already ship in this
/// crate (the former `/itl/*` surface). A product binary MAY merge these with
/// its own `SezsService::routes`.
#[cfg(all(feature = "server", feature = "onboard"))]
pub fn builtin_inbox_routes(inbox: crate::onboard_authz::frost_http::SharedInbox) -> Router {
    crate::frost::product_routes(inbox)
}

#[cfg(test)]
mod tests {
    use super::{IndexerProcess, canonical_path};

    #[test]
    fn aliases_itl_to_sezs() {
        assert_eq!(canonical_path("/itl/session/me"), "/sezs/session/me");
        assert_eq!(canonical_path("/itl"), "/sezs");
        assert_eq!(canonical_path("/sezs/session/me"), "/sezs/session/me");
        assert_eq!(canonical_path("/health"), "/health");
        assert_eq!(canonical_path("/frost/jobs"), "/sezs/frost/jobs");
        assert_eq!(canonical_path("/session/me"), "/sezs/session/me");
        assert_eq!(canonical_path("/door/consume"), "/sezs/door/consume");
        assert_eq!(canonical_path("/itl/frost/jobs"), "/sezs/frost/jobs");
    }

    #[test]
    fn indexer_formula_url_shape() {
        struct Stub;
        impl super::IndexerProcess for Stub {}
        let s = Stub;
        assert_eq!(
            s.formula_url(
                "https://idx.example",
                "morocco-1",
                "terp1contract",
                "EventView",
                &[]
            ),
            "https://idx.example/morocco-1/contract/terp1contract/EventView"
        );
        assert_eq!(
            s.formula_url(
                "http://127.0.0.1:9999",
                "x",
                "a",
                "Roles",
                &[("limit", "2")]
            ),
            "http://127.0.0.1:9999/x/contract/a/Roles?limit=2"
        );
    }
}
