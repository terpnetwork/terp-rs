//! Collections this runtime may mint on. Empty means broadcast stays idle.
//!
//! `DREGG_COLLECTIONS` is a comma or whitespace separated list of `terp1` addresses.
//! Otherwise `~/.terp/dregg-svg-collections` is one address per line.
//! The legacy cell id `svg` is not a contract and is never a pay target.
//!
//! A configured address is admitted only after `simulate` accepts an empty-funds
//! SVG mint from the runtime signer. A contract reject stays out of the pay set.
//! An account or LCD miss is unproven: pay stays closed, and the address is not
//! labeled as a bad contract.

use std::path::PathBuf;

pub fn watched() -> Vec<String> {
    if let Ok(raw) = std::env::var("DREGG_COLLECTIONS") {
        let list = parse_list(&raw);
        if !list.is_empty() {
            return list;
        }
    }
    let path = collections_file();
    match std::fs::read_to_string(&path) {
        Ok(text) => parse_list(&text),
        Err(_) => Vec::new(),
    }
}

pub fn parse_list(raw: &str) -> Vec<String> {
    let mut out = Vec::new();
    for part in raw.split(|c: char| c == ',' || c.is_whitespace()) {
        let a = part.trim();
        if a.starts_with("terp1") && !out.iter().any(|e: &String| e == a) {
            out.push(a.to_string());
        }
    }
    out
}

/// The only watched contract, when the set has exactly one.
pub fn sole() -> Option<String> {
    let w = watched();
    if w.len() == 1 {
        Some(w.into_iter().next().unwrap())
    } else {
        None
    }
}

pub fn contains(addr: &str) -> bool {
    screen().admitted.iter().any(|a| a == addr)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CollectionNote {
    pub address: String,
    pub reason: String,
}

/// Configured contracts split by a simulate of the empty-funds mint.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Screen {
    pub admitted: Vec<String>,
    pub rejected: Vec<CollectionNote>,
    pub unproven: Vec<CollectionNote>,
}

impl Screen {
    fn empty() -> Self {
        Self {
            admitted: Vec::new(),
            rejected: Vec::new(),
            unproven: Vec::new(),
        }
    }
}

/// Admitted contracts. Broadcast and pay-targets use this set, not the raw file.
pub fn admitted() -> Vec<String> {
    screen().admitted
}

/// Process-local stand-in for simulate. The fed never calls this.
/// `Some` skips LCD and treats those addresses as admitted.
pub fn set_admission_override(addrs: Option<Vec<String>>) {
    let mut g = OVERRIDE.lock().unwrap_or_else(|p| p.into_inner());
    *g = addrs;
}

static OVERRIDE: std::sync::Mutex<Option<Vec<String>>> = std::sync::Mutex::new(None);

/// Cached for 20s so health does not simulate on every request.
pub fn screen() -> Screen {
    if let Some(addrs) = OVERRIDE.lock().unwrap_or_else(|p| p.into_inner()).clone() {
        return Screen {
            admitted: addrs,
            rejected: Vec::new(),
            unproven: Vec::new(),
        };
    }
    let configured = watched();
    if configured.is_empty() {
        return Screen::empty();
    }
    let key = configured.join(",");
    if let Some(hit) = cache_get(&key) {
        return hit;
    }
    let mut screen = Screen::empty();
    for addr in configured {
        match crate::mint_tx::simulate_empty_mint(&addr) {
            crate::mint_tx::SimVerdict::Accepted => screen.admitted.push(addr),
            crate::mint_tx::SimVerdict::Rejected(reason) => {
                screen.rejected.push(CollectionNote { address: addr, reason })
            }
            crate::mint_tx::SimVerdict::Unproven(reason) => {
                screen.unproven.push(CollectionNote { address: addr, reason })
            }
        }
    }
    cache_put(key, screen.clone());
    screen
}

fn cache_get(key: &str) -> Option<Screen> {
    let g = CACHE.lock().unwrap_or_else(|p| p.into_inner());
    let (k, at, screen) = g.as_ref()?;
    if k == key && at.elapsed() < std::time::Duration::from_secs(20) {
        Some(screen.clone())
    } else {
        None
    }
}

fn cache_put(key: String, screen: Screen) {
    let mut g = CACHE.lock().unwrap_or_else(|p| p.into_inner());
    *g = Some((key, std::time::Instant::now(), screen));
}

static CACHE: std::sync::Mutex<Option<(String, std::time::Instant, Screen)>> =
    std::sync::Mutex::new(None);

fn collections_file() -> PathBuf {
    if let Ok(p) = std::env::var("DREGG_COLLECTIONS_FILE") {
        if !p.is_empty() {
            return PathBuf::from(p);
        }
    }
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    PathBuf::from(home).join(".terp/dregg-svg-collections")
}

/// Resolve the contract for one checkout.
/// One admitted address is the default. Several require an explicit `terp1`.
/// An empty or unproven set does not fall back to the legacy id `svg`.
pub fn resolve(requested: Option<&str>) -> Result<String, String> {
    if watched().is_empty() {
        return Err("checkout closed: collections_unset".into());
    }
    let screen = screen();
    if let Some(c) = requested.map(str::trim).filter(|s| s.starts_with("terp1")) {
        if screen.admitted.iter().any(|a| a == c) {
            return Ok(c.to_string());
        }
        if screen.rejected.iter().any(|n| n.address == c) {
            return Err("checkout closed: collection_rejected".into());
        }
        if screen.unproven.iter().any(|n| n.address == c) {
            return Err("checkout closed: collection_unproven".into());
        }
        return Err("collection is not in the watched set".into());
    }
    if screen.admitted.is_empty() {
        if screen.rejected.is_empty() {
            Err("checkout closed: collection_unproven".into())
        } else {
            Err("checkout closed: collection_rejected".into())
        }
    } else if screen.admitted.len() == 1 {
        Ok(screen.admitted[0].clone())
    } else {
        Err("collection query required when several collections are watched".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_set_does_not_resolve_svg() {
        let _g = crate::checkout::minter_env_lock();
        unsafe { std::env::remove_var("DREGG_COLLECTIONS") };
        unsafe { std::env::remove_var("DREGG_COLLECTIONS_ADMIT") };
        unsafe { std::env::set_var("DREGG_COLLECTIONS_FILE", "/tmp/dregg-collections-absent") };
        let err = resolve(None).unwrap_err();
        unsafe { std::env::remove_var("DREGG_COLLECTIONS_FILE") };
        assert_eq!(err, "checkout closed: collections_unset");
    }

    #[test]
    fn admitted_singleton_resolves() {
        let _g = crate::checkout::minter_env_lock();
        unsafe { std::env::set_var("DREGG_COLLECTIONS", "terp1testcollectionxxxxxxxxxxxxxxxxxx") };
        set_admission_override(Some(vec!["terp1testcollectionxxxxxxxxxxxxxxxxxx".into()]));
        assert_eq!(
            resolve(None).unwrap(),
            "terp1testcollectionxxxxxxxxxxxxxxxxxx"
        );
        unsafe { std::env::remove_var("DREGG_COLLECTIONS") };
        set_admission_override(None);
    }

    #[test]
    fn parse_keeps_terp1_only() {
        let v = parse_list("svg, terp1aaa\nterp1bbb terp1aaa no");
        assert_eq!(v, vec!["terp1aaa".to_string(), "terp1bbb".to_string()]);
    }
}
