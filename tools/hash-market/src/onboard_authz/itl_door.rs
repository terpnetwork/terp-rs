//! Door consume. An usher on the list may mark one paid ticket used.
//! Removal drops that right immediately. Unpaid and a second use are refused.
//! This is the local stand-in for cw-ave VerifyConsume. It does not list guests.

use super::http::HttpResponse;
use super::itl_session::session_from_cookie;
use serde::Deserialize;
use serde_json::json;
use std::collections::BTreeSet;
use std::sync::{Mutex, OnceLock};

#[derive(Default)]
struct Book {
    ushers: BTreeSet<String>,
    paid: BTreeSet<String>,
    spent: BTreeSet<String>,
}

fn book() -> &'static Mutex<Book> {
    static B: OnceLock<Mutex<Book>> = OnceLock::new();
    B.get_or_init(|| Mutex::new(Book::default()))
}

pub fn reset_for_test() {
    if let Ok(mut g) = book().lock() {
        *g = Book::default();
    }
}

impl Book {
    fn consume(&mut self, usher: &str, ticket: &str) -> Result<(), &'static str> {
        if !self.ushers.contains(usher) {
            return Err("not_usher");
        }
        if ticket.trim().is_empty() {
            return Err("unpaid");
        }
        if !self.paid.contains(ticket) {
            return Err("unpaid");
        }
        if !self.spent.insert(ticket.to_string()) {
            return Err("nullifier_spent");
        }
        Ok(())
    }
}

fn err(status: u16, error: &str) -> HttpResponse {
    HttpResponse {
        status,
        body: json!({"ok": false, "error": error}).to_string(),
    }
}

fn ok(extra: serde_json::Value) -> HttpResponse {
    HttpResponse {
        status: 200,
        body: json!({"ok": true, "result": extra}).to_string(),
    }
}

#[derive(Deserialize)]
struct UsherBody {
    id: String,
    action: String,
}

#[derive(Deserialize)]
struct TicketBody {
    ticket: String,
}

#[derive(Deserialize)]
struct ConsumeBody {
    ticket: String,
}

pub fn try_handle(
    method: &str,
    path: &str,
    body: &str,
    cookie: Option<&str>,
) -> Option<HttpResponse> {
    let path = path.split('?').next().unwrap_or(path);
    if method != "POST" {
        return None;
    }
    if path == "/itl/door/paid" {
        let parsed: TicketBody = serde_json::from_str(body).ok()?;
        let mut g = book().lock().ok()?;
        if parsed.ticket.trim().is_empty() {
            return Some(err(400, "unpaid"));
        }
        g.paid.insert(parsed.ticket);
        return Some(ok(json!("paid")));
    }
    if path == "/itl/door/ushers" {
        let parsed: UsherBody = serde_json::from_str(body).ok()?;
        let id = parsed.id.trim().to_string();
        if id.is_empty() {
            return Some(err(400, "bad_seat"));
        }
        let mut g = book().lock().ok()?;
        if parsed.action == "add" {
            g.ushers.insert(id);
        } else if parsed.action == "remove" {
            g.ushers.remove(&id);
        } else {
            return Some(err(400, "bad_seat"));
        }
        return Some(ok(json!(g.ushers.iter().cloned().collect::<Vec<_>>())));
    }
    if path == "/itl/door/consume" {
        let view = session_from_cookie(cookie)?;
        if view.role != "usher" {
            return Some(err(403, "not_usher"));
        }
        let parsed: ConsumeBody = match serde_json::from_str(body) {
            Ok(p) => p,
            Err(_) => return Some(err(400, "bad_json")),
        };
        let mut g = book().lock().ok()?;
        return Some(match g.consume(&view.login_id, parsed.ticket.trim()) {
            Ok(()) => ok(json!("consumed")),
            Err(e) => err(if e == "not_usher" { 403 } else { 400 }, e),
        });
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::onboard_authz::itl_session::{self, mint_test_cookie};

    #[test]
    fn unpaid_double_and_removed_usher_are_refused() {
        super::reset_for_test();
        let admin = mint_test_cookie("admin", "desk");
        let usher = mint_test_cookie("usher", "ada");
        assert_eq!(
            try_handle(
                "POST",
                "/itl/door/ushers",
                r#"{"id":"ada","action":"add"}"#,
                Some(&admin)
            )
            .unwrap()
            .status,
            200
        );
        let unpaid = try_handle(
            "POST",
            "/itl/door/consume",
            r#"{"ticket":"tkt-1"}"#,
            Some(&usher),
        )
        .unwrap();
        assert_eq!(unpaid.status, 400);
        assert!(unpaid.body.contains("unpaid"));
        assert_eq!(
            try_handle(
                "POST",
                "/itl/door/paid",
                r#"{"ticket":"tkt-1"}"#,
                Some(&admin)
            )
            .unwrap()
            .status,
            200
        );
        assert_eq!(
            try_handle(
                "POST",
                "/itl/door/consume",
                r#"{"ticket":"tkt-1"}"#,
                Some(&usher),
            )
            .unwrap()
            .status,
            200
        );
        let again = try_handle(
            "POST",
            "/itl/door/consume",
            r#"{"ticket":"tkt-1"}"#,
            Some(&usher),
        )
        .unwrap();
        assert!(again.body.contains("nullifier_spent"));
        try_handle(
            "POST",
            "/itl/door/ushers",
            r#"{"id":"ada","action":"remove"}"#,
            Some(&admin),
        );
        let gone = try_handle(
            "POST",
            "/itl/door/consume",
            r#"{"ticket":"tkt-2"}"#,
            Some(&usher),
        )
        .unwrap();
        assert_eq!(gone.status, 403);
        assert!(gone.body.contains("not_usher"));
    }
}
