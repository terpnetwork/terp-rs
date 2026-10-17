//! Round-trip every corpus template (live mainnet code-71 collections + launchpad
//! examples) and assert the Rust encoding is byte-identical to the JS encoding.
use cw_svg::{compute_template_slots, VariableDef};
use std::fs;

#[test]
fn corpus_roundtrip_and_js_parity() {
    let dir = std::env::var("SVP_CORPUS").unwrap_or_else(|_| concat!(env!("CARGO_MANIFEST_DIR"), "/fixtures/corpus").to_string());
    let mut n = 0;
    for e in fs::read_dir(&dir).unwrap() {
        let p = e.unwrap().path();
        if p.extension().and_then(|x| x.to_str()) != Some("json") { continue; }
        let v: serde_json::Value = serde_json::from_slice(&fs::read(&p).unwrap()).unwrap();
        let tpl = v["svg_template"].as_str().unwrap().to_string();
        let vars_v = if v.get("variables").is_some() { v["variables"].clone() } else { v["dictionary"]["vars"].clone() };
        let vars: Vec<VariableDef> = serde_json::from_value(vars_v).unwrap();
        let packed = cw_svg_pack::pack(&tpl, &vars).unwrap();
        let (t2, v2) = cw_svg_pack::unpack(&packed).unwrap();
        assert_eq!(t2, tpl, "{p:?} template");
        assert_eq!(v2, vars, "{p:?} vars");
        assert_eq!(cw_svg_pack::unpack_vars(&packed).unwrap(), vars);
        if let Some(sl) = v.get("template_slots") {
            let got = serde_json::to_value(compute_template_slots(&t2, &v2)).unwrap();
            assert_eq!(&got, sl, "{p:?} slots");
        }
        let js = p.parent().unwrap().join("../out").join(p.file_name().unwrap()).with_extension("svp");
        assert_eq!(fs::read(&js).unwrap(), packed, "{p:?} JS/Rust byte parity");
        // Corruption must error, never panic.
        for cut in [5usize, packed.len() / 2, packed.len() - 1] {
            let _ = cw_svg_pack::unpack(&packed[..cut]);
        }
        n += 1;
    }
    assert!(n >= 14, "corpus too small: {n}");
}

#[test]
fn rejects_bombs() {
    // COPY of 2^20 bytes from a 1-byte literal must hit MAX_OUT, not allocate.
    let mut b = vec![0x53, 0x56, 0x50, 0x01, 0, 1, 1, b'a', 3, 1];
    b.extend_from_slice(&[0x80, 0x80, 0x40, 0]);
    assert!(cw_svg_pack::unpack(&b).is_err());
}
