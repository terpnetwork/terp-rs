//! Contract labelling spec (SPEC §3). Labels are `terp/lc/<role>/<target>/v<major>`:
//!
//! * `role` — one of [`ROLES`]
//! * `target` — target chain slug: `[a-z0-9][a-z0-9-]{1,47}` (e.g. `zcash-main`,
//!   `zcash-regtest`, `zakura-crosslink`)
//! * `v<major>` — contract major version, `v1`..`v999`
use crate::error::ContractError;

pub const LABEL_PREFIX: &str = "terp/lc/";
pub const ROLES: &[&str] = &["registry", "rotation", "aggregator", "client", "vault"];

fn bad(label: &str, reason: &str) -> ContractError {
    ContractError::InvalidLabel {
        label: label.to_string(),
        reason: reason.to_string(),
    }
}

pub fn is_slug(s: &str, max: usize) -> bool {
    let b = s.as_bytes();
    !b.is_empty()
        && b.len() <= max
        && (b[0].is_ascii_lowercase() || b[0].is_ascii_digit())
        && b.iter()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || *c == b'-')
}

/// Validate `label` and require `role` (e.g. `registry`). Returns the target slug.
pub fn validate_label<'a>(label: &'a str, role: &str) -> Result<&'a str, ContractError> {
    let rest = label
        .strip_prefix(LABEL_PREFIX)
        .ok_or_else(|| bad(label, "must start with terp/lc/"))?;
    let parts: Vec<&str> = rest.split('/').collect();
    if parts.len() != 3 {
        return Err(bad(label, "expected terp/lc/<role>/<target>/v<major>"));
    }
    if !ROLES.contains(&parts[0]) {
        return Err(bad(label, "unknown role"));
    }
    if parts[0] != role {
        return Err(bad(label, "role does not match this contract"));
    }
    if !is_slug(parts[1], 48) || parts[1].len() < 2 {
        return Err(bad(label, "target must be a lowercase slug (2..=48)"));
    }
    let v = parts[2]
        .strip_prefix('v')
        .ok_or_else(|| bad(label, "version must be v<major>"))?;
    match v.parse::<u16>() {
        Ok(n) if (1..=999).contains(&n) && !v.starts_with('0') => Ok(parts[1]),
        _ => Err(bad(label, "version must be v1..v999")),
    }
}

/// Client ids: lowercase slug up to 64 chars (`zcash-regtest-0`, `08-wasm-7`).
pub fn validate_client_id(id: &str) -> Result<(), ContractError> {
    if id.len() >= 3 && is_slug(id, 64) {
        Ok(())
    } else {
        Err(ContractError::InvalidClientId(id.to_string()))
    }
}
