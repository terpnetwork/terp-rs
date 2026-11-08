//! Curated cw721-svg hopalong collection: on-chain seed → fractal coordinates.
//! Not a 1×1 vanilla placeholder.

/// SVG template with `${VAR}` slots. Contract binds vars from `seed` entropy.
pub const SVG_TEMPLATE: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="-80 -80 160 160" width="512" height="512">
  <rect x="-80" y="-80" width="160" height="160" fill="#08090d"/>
  <circle cx="${CX}" cy="${CY}" r="1.2" fill="#b1ebeb" opacity="0.9"/>
  <g fill="none" stroke="#bd93f9" stroke-width="0.35" opacity="0.85">
    <path d="M ${CX} ${CY} q ${A} ${B} ${DX} ${DY}"/>
  </g>
  <desc>hopalong a=${A} b=${B} seed-coords</desc>
</svg>"##;

pub fn collection_variables() -> serde_json::Value {
    serde_json::json!([
        {"name": "A", "kind": {"range": {"min": "-4", "max": "1", "precision": 2}}, "entropy": "seed % 10000"},
        {"name": "B", "kind": {"range": {"min": "-1", "max": "2", "precision": 2}}, "entropy": "(seed >> 8) % 10000"},
        {"name": "CX", "kind": {"range": {"min": "-70", "max": "70", "precision": 2}}, "entropy": "(seed >> 16) % 10000"},
        {"name": "CY", "kind": {"range": {"min": "-70", "max": "70", "precision": 2}}, "entropy": "(seed >> 24) % 10000"},
        {"name": "DX", "kind": {"range": {"min": "-40", "max": "40", "precision": 2}}, "entropy": "(seed >> 32) % 10000"},
        {"name": "DY", "kind": {"range": {"min": "-40", "max": "40", "precision": 2}}, "entropy": "(seed >> 40) % 10000"}
    ])
}

pub fn is_vanilla_placeholder(svg: &str) -> bool {
    let t = svg.trim();
    t.contains("width=\"1\"") && t.contains("height=\"1\"") && !t.contains("hopalong")
}

/// Scan `${NAME}` in `template` against `variables[].name`. Byte offsets, no hand slots.
pub fn compute_template_slots(template: &str, variables: &[serde_json::Value]) -> Vec<serde_json::Value> {
    let names: Vec<&str> = variables
        .iter()
        .filter_map(|v| v.get("name").and_then(|n| n.as_str()))
        .collect();
    let mut slots = Vec::new();
    let b = template.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'$' && i + 1 < b.len() && b[i + 1] == b'{' {
            if let Some(rel) = template[i + 2..].find('}') {
                let name = &template[i + 2..i + 2 + rel];
                let end = i + 2 + rel + 1;
                if let Some(var_idx) = names.iter().position(|n| *n == name) {
                    slots.push(serde_json::json!({
                        "start": i,
                        "end": end,
                        "var_idx": var_idx
                    }));
                }
                i = end;
                continue;
            }
        }
        i += 1;
    }
    slots
}

/// cw721-svg instantiate body. `template_slots` is a scan of SVG_TEMPLATE, not a hand list.
pub fn collection_instantiate(owner: &str) -> serde_json::Value {
    let vars = collection_variables();
    let vars_clean: Vec<serde_json::Value> = vars
        .as_array()
        .map(|a| {
            a.iter()
                .map(|v| serde_json::json!({ "name": v["name"], "kind": v["kind"] }))
                .collect()
        })
        .unwrap_or_default();
    let template_slots = compute_template_slots(SVG_TEMPLATE, &vars_clean);
    serde_json::json!({
        "name": "lab",
        "symbol": "LAB",
        "minter": owner,
        "creator": owner,
        "withdraw_address": owner,
        "collection_info_extension": {
            "seed": "0000000000000000000000000000000000000000000000000000000000000000",
            "svg_template": SVG_TEMPLATE,
            "template_slots": template_slots,
            "variables": vars_clean,
            "price_tiers": [{
                "until_count": 1000,
                "price": {"denom": "uterp", "amount": crate::prices::NATIVE_AMOUNT.to_string()}
            }],
            "total": 10000,
            "mint_start_time": null,
            "mint_end_time": null,
            "whitelist": null,
            "require_auth": false
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn template_slots_scan_covers_all_placeholders() {
        let vars = collection_variables();
        let arr = vars.as_array().unwrap();
        let slots = compute_template_slots(SVG_TEMPLATE, arr);
        let n_ph = SVG_TEMPLATE.matches("${").count();
        assert_eq!(slots.len(), n_ph);
        assert!(n_ph >= 6);
        let body = collection_instantiate("terp1owner");
        assert!(body["collection_info_extension"]["template_slots"]
            .as_array()
            .map(|a| !a.is_empty())
            .unwrap_or(false));
    }
}
