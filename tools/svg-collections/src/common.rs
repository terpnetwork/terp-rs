use cw_svg::{TokenParam, VariableDef};
use sha2::{Digest, Sha256};

pub fn sha256_hex(b: &[u8]) -> String {
    Sha256::digest(b).iter().map(|x| format!("{:02x}", x)).collect()
}

/// Same rule as cw721-svg `derive_entropy`.
pub fn derive_entropy(token_seed: &[u8], var_idx: usize) -> u64 {
    let mut h = Sha256::new();
    h.update(token_seed);
    h.update((var_idx as u64).to_le_bytes());
    u64::from_le_bytes(h.finalize()[0..8].try_into().unwrap())
}

/// Same rule as cw721-svg mint: sha256(tid_le || seed || height_le).
pub fn token_params(vars: &[VariableDef], tid: u64, seed: &[u8], height: u64) -> Vec<TokenParam> {
    let mut h = Sha256::new();
    h.update(tid.to_le_bytes());
    h.update(seed);
    h.update(height.to_le_bytes());
    let ts = h.finalize();
    vars.iter()
        .enumerate()
        .map(|(i, v)| cw_svg::resolve_variable(&ts, i, v, derive_entropy))
        .collect()
}

/// The contract's own render (`cw721_svg::contract::render_params`), incl. scribe derivation.
pub fn render(template: &str, vars: &[VariableDef], params: &[TokenParam]) -> String {
    let slots = cw_svg::compute_template_slots(template, vars);
    let layout = cw721_svg::contract::scribe_layout(vars).expect("scribe layout");
    cw721_svg::contract::render_params(template, &slots, params, layout, None, None)
}

#[derive(Debug, serde::Serialize)]
pub struct BuildReport {
    pub template_bytes: usize,
    pub packed_bytes: usize,
    pub packed_sha256: String,
    pub instantiate_msg_bytes: usize,
    pub est_create_gas_fit: u64,
    pub json_equiv_msg_bytes: usize,
    pub est_mint_gas_fit: u64,
    pub fit_note: &'static str,
}

/// Run the on-chain checks, pack, round-trip, and estimate gas with the page's GAS_FIT.
pub fn check_and_pack(template: &str, vars: &[VariableDef]) -> Result<(Vec<u8>, BuildReport), String> {
    cw_svg::validate_variables(vars).map_err(|e| format!("vars: {e}"))?;
    let slots = cw_svg::compute_template_slots(template, vars);
    cw_svg::validate_template_slots(template, vars, &slots).map_err(|e| format!("slots: {e}"))?;
    cw_svg::validate_animated_paint(template).map_err(|e| format!("paint: {e}"))?;
    cw_svg::links::validate_links(template, vars).map_err(|e| format!("links: {e}"))?;
    cw721_svg::contract::scribe_layout(vars).map_err(|e| format!("scribe: {e}"))?;
    if template.len() > cw_svg_pack::MAX_OUT {
        return Err(format!("template {} > MAX_OUT {}", template.len(), cw_svg_pack::MAX_OUT));
    }
    let packed = cw_svg_pack::pack(template, vars).map_err(|e| format!("pack: {e:?}"))?;
    let (t2, v2) = cw_svg_pack::unpack(&packed).map_err(|e| format!("unpack: {e:?}"))?;
    if t2 != template || v2 != vars {
        return Err("round-trip mismatch".into());
    }
    use base64::Engine;
    let b64 = base64::engine::general_purpose::STANDARD.encode(&packed);
    let inst = serde_json::json!({"name":"collection-name-xxxxxxxx","symbol":"XXXX",
        "collection_info_extension":{"seed":"A".repeat(44),"packed":b64,"price_tiers":[],
        "total":1000,"mint_start_time":null,"mint_end_time":null,"whitelist":null},
        "minter":null,"creator":null,"withdraw_address":null});
    let inst_len = serde_json::to_vec(&inst).unwrap().len();
    let json_equiv = serde_json::to_vec(&serde_json::json!({"svg_template":template,"variables":vars})).unwrap().len();
    Ok((packed.clone(), BuildReport {
        template_bytes: template.len(),
        packed_bytes: packed.len(),
        packed_sha256: sha256_hex(&packed),
        instantiate_msg_bytes: inst_len,
        est_create_gas_fit: (207639.0 + 63.39 * inst_len as f64) as u64,
        json_equiv_msg_bytes: json_equiv,
        est_mint_gas_fit: (140757.0 + 4.01 * json_equiv as f64) as u64,
        fit_note: "svp.js GAS_FIT (fit on 7 corpus templates up to ~15 KB); larger inputs are extrapolation, see measured numbers in REPORT.md",
    }))
}

pub fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}
