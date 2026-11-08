//! R12: on-chain UI theme tokens for the Headstash claim page.
//! Identical copy in cw-headstash and cw-headstash-frontend (keep in sync; the browser
//! re-validates with the same rules in tools/headstash/frontend/v2lib.js validateTheme).
//! Only whitelisted token names, only `#RGB` / `#RRGGBB` values and an integer radius are
//! accepted, so a stored theme can never carry CSS syntax (`;`, `}`, `url(`, `expression(`).
use cosmwasm_schema::cw_serde;
use cosmwasm_std::StdError;
use std::collections::BTreeMap;

pub const THEME_TOKENS: [&str; 10] = ["bg", "surface", "surfaceAlt", "text", "textMuted", "border", "borderStrong", "accent", "onAccent", "focus"];
pub const RADIUS_MAX: u8 = 24;

#[cw_serde]
#[derive(Default)]
#[serde(deny_unknown_fields)]
pub struct Theme {
    /// token name -> colour for `:root[data-theme=light]`
    #[serde(default)]
    pub light: BTreeMap<String, String>,
    /// token name -> colour for `:root[data-theme=dark]`
    #[serde(default)]
    pub dark: BTreeMap<String, String>,
    /// corner radius in px (0..=24); unset = renderer default
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub radius_px: Option<u8>,
}

fn valid_colour(v: &str) -> bool {
    let b = v.as_bytes();
    (b.len() == 4 || b.len() == 7) && b[0] == b'#' && b[1..].iter().all(|c| c.is_ascii_hexdigit())
}

pub fn validate_theme(t: &Theme) -> Result<(), StdError> {
    let mut n = 0usize;
    for (mode, m) in [("light", &t.light), ("dark", &t.dark)] {
        if m.len() > THEME_TOKENS.len() {
            return Err(StdError::msg(format!("theme.{mode}: too many tokens")));
        }
        for (k, v) in m {
            if !THEME_TOKENS.contains(&k.as_str()) {
                return Err(StdError::msg(format!("theme.{mode}: unknown token (allowed: {})", THEME_TOKENS.join(","))));
            }
            if !valid_colour(v) {
                return Err(StdError::msg(format!("theme.{mode}.{k}: colour must be #RGB or #RRGGBB")));
            }
            n += 1;
        }
    }
    if let Some(r) = t.radius_px {
        if r > RADIUS_MAX {
            return Err(StdError::msg(format!("theme.radius_px: max {RADIUS_MAX}")));
        }
    }
    if n == 0 {
        return Err(StdError::msg("theme: no tokens"));
    }
    Ok(())
}

/// Apricot Glow (https://permissionless.money/alpha/wd-colours/apricot-glow.html).
pub fn apricot_glow() -> Theme {
    let m = |v: [(&str, &str); 10]| v.iter().map(|(k, c)| (k.to_string(), c.to_string())).collect();
    Theme {
        light: m([("bg", "#FEF4E7"), ("surface", "#FFFBF6"), ("surfaceAlt", "#F9EAD7"), ("text", "#473926"), ("textMuted", "#675336"),
                  ("border", "#E6D6C1"), ("borderStrong", "#E8B773"), ("accent", "#896D46"), ("onAccent", "#FFFFFF"), ("focus", "#896D46")]),
        dark: m([("bg", "#170D02"), ("surface", "#241704"), ("surfaceAlt", "#332612"), ("text", "#F4EADD"), ("textMuted", "#C2B3A0"),
                 ("border", "#41331F"), ("borderStrong", "#896D46"), ("accent", "#E8B773"), ("onAccent", "#271F14"), ("focus", "#E8B773")]),
        radius_px: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn one(mode: &str, k: &str, v: &str) -> Theme {
        let mut t = Theme::default();
        let m = if mode == "light" { &mut t.light } else { &mut t.dark };
        m.insert(k.into(), v.into());
        t
    }
    #[test]
    fn accepts() {
        validate_theme(&apricot_glow()).unwrap();
        validate_theme(&one("light", "bg", "#fff")).unwrap();
        validate_theme(&one("dark", "focus", "#AbCdEf")).unwrap();
        let mut t = one("light", "bg", "#fff");
        t.radius_px = Some(24);
        validate_theme(&t).unwrap();
        assert_eq!(apricot_glow().light.len(), 10);
        assert_eq!(apricot_glow().dark.len(), 10);
    }
    #[test]
    fn rejects() {
        let long = format!("#{}", "a".repeat(300));
        for (k, v) in [
            ("bg", "red"), ("bg", "red;}body{"), ("bg", "#fff;}body{background:url(x)}"), ("bg", "url(https://evil/x.png)"),
            ("bg", "expression(alert(1))"), ("bg", long.as_str()), ("bg", "#ffff"), ("bg", "#12345g"), ("bg", " #ffffff"),
            ("bg", "#fff\n"), ("bg", ""), ("bg", "#"), ("bg", "var(--x)"), ("bg", "#fff/**/"), ("bg", "#ＦＦＦ"),
            ("bg;}x{", "#fff"), ("color", "#fff"), ("Bg", "#fff"), ("", "#fff"),
        ] {
            for mode in ["light", "dark"] {
                assert!(validate_theme(&one(mode, k, v)).is_err(), "{mode} {k:?}={v:?}");
            }
        }
        assert!(validate_theme(&Theme::default()).is_err());
        let mut t = one("light", "bg", "#fff");
        t.radius_px = Some(25);
        assert!(validate_theme(&t).is_err());
    }
    #[test]
    fn json_shape_and_unknown_fields() {
        let t: Theme = cosmwasm_std::from_json(br##"{"light":{"bg":"#fff"}}"##).unwrap();
        validate_theme(&t).unwrap();
        assert_eq!(cosmwasm_std::to_json_string(&t).unwrap(), r##"{"light":{"bg":"#fff"},"dark":{}}"##);
        assert!(cosmwasm_std::from_json::<Theme>(br##"{"light":{"bg":"#fff"},"font":"x"}"##).is_err());
        assert!(cosmwasm_std::from_json::<Theme>(br##"{"light":{"bg":"#fff"},"radius_px":"4px;}"}"##).is_err());
        assert!(cosmwasm_std::from_json::<Theme>(br##"{"light":{"bg":"#fff"},"radius_px":300}"##).is_err());
    }
}

// ---- cw-headstash storage + handlers (not in the frontend copy) ----
pub const THEME: cw_storage_plus::Item<Theme> = cw_storage_plus::Item::new("ui_theme");

pub fn execute_set_theme(deps: cosmwasm_std::DepsMut, info: cosmwasm_std::MessageInfo, theme: Option<Theme>) -> Result<cosmwasm_std::Response, StdError> {
    cw_ownable::assert_owner(deps.storage, &info.sender).map_err(|e| StdError::msg(e.to_string()))?;
    let r = cosmwasm_std::Response::new().add_attribute("action", "set_theme");
    match theme {
        Some(t) => {
            validate_theme(&t)?;
            let n = t.light.len() + t.dark.len();
            THEME.save(deps.storage, &t)?;
            Ok(r.add_attribute("tokens", n.to_string()))
        }
        None => {
            THEME.remove(deps.storage);
            Ok(r.add_attribute("tokens", "none"))
        }
    }
}

pub fn query_theme(deps: cosmwasm_std::Deps) -> cosmwasm_std::StdResult<cosmwasm_std::Binary> {
    cosmwasm_std::to_json_binary(&THEME.may_load(deps.storage)?)
}

#[cfg(test)]
mod contract_tests {
    use super::*;
    use cosmwasm_std::testing::{message_info, mock_dependencies};
    #[test]
    fn owner_sets_validates_queries_clears() {
        let mut deps = mock_dependencies();
        let owner = deps.api.addr_make("owner");
        let other = deps.api.addr_make("other");
        cw_ownable::initialize_owner(&mut deps.storage, &deps.api, Some(owner.as_str())).unwrap();
        let q = |d: cosmwasm_std::Deps| cosmwasm_std::from_json::<Option<Theme>>(query_theme(d).unwrap()).unwrap();
        assert_eq!(q(deps.as_ref()), None);
        assert!(execute_set_theme(deps.as_mut(), message_info(&other, &[]), Some(apricot_glow())).is_err());
        let mut bad = apricot_glow();
        bad.light.insert("bg".into(), "red;}body{".into());
        assert!(execute_set_theme(deps.as_mut(), message_info(&owner, &[]), Some(bad)).is_err());
        assert_eq!(q(deps.as_ref()), None);
        let r = execute_set_theme(deps.as_mut(), message_info(&owner, &[]), Some(apricot_glow())).unwrap();
        assert_eq!(r.attributes[1].value, "20");
        assert_eq!(q(deps.as_ref()), Some(apricot_glow()));
        execute_set_theme(deps.as_mut(), message_info(&owner, &[]), None).unwrap();
        assert_eq!(q(deps.as_ref()), None);
    }
}
