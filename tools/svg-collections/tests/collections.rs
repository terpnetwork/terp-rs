use svg_collections::{ascii, cid, common, marain};

#[test]
fn glyph_table_matches_json_and_banks_w() {
    let j: serde_json::Value = serde_json::from_str(marain::GLYPHS_JSON).unwrap();
    let rows = j["glyphs"].as_array().unwrap();
    assert_eq!(rows.len(), marain::GLYPHS.len());
    for r in rows {
        let k = r["key"].as_str().unwrap();
        let bits = u16::from_str_radix(r["bits"].as_str().unwrap(), 2).unwrap();
        assert!(marain::GLYPHS.contains(&(k, bits)), "{k}");
    }
    let w = marain::GLYPHS.iter().find(|g| g.0 == "w").unwrap().1;
    assert_eq!(w, 0b100111100);
    assert_eq!((0..9).fold(0u16, |a, i| a | (((w >> i) & 1) << (8 - i))), 121); // Banks' 121 (LSB-first)
}

#[test]
fn themes_are_valid_and_32() {
    let t = marain::themes();
    assert_eq!(t.len(), 32);
    for x in &t { marain::validate_text(x).unwrap_or_else(|e| panic!("{x}: {e}")); }
}

#[test]
fn marain_scribe_template_passes_chain_checks() {
    let vars = marain::variables(&marain::themes());
    let t = marain::template();
    let (packed, rep) = common::check_and_pack(&t, &vars).unwrap();
    assert!(rep.packed_bytes < 2048, "{rep:?}");
    for tid in 0..5 {
        let mut p = common::token_params(&vars, tid, b"seed", 7);
        let before = common::render(&t, &vars, &p);
        cw_svg::validate_animated_paint(&before).unwrap();
        assert!(!before.contains("${") && before.contains(&format!(">{}</text>", p[0].value)));
        p[0].value = "every orbit is a promise".into();
        let after = common::render(&t, &vars, &p);
        let (cells, _) = marain::derived("every orbit is a promise");
        assert!(after.contains(&format!("values=\"{}\"", cells[8])));
        assert_ne!(before, after);
    }
    assert_eq!(cw_svg_pack::unpack_vars(&packed).unwrap(), vars);
}

#[test]
fn animated_fill_would_be_rejected() {
    let bad = "<svg><circle r=\"3\" fill=\"none\"><animate attributeName=\"fill\" values=\"red;blue\" dur=\"1s\"/></circle></svg>";
    assert!(cw_svg::validate_animated_paint(bad).is_err());
}

#[test]
fn sanitizer_is_deterministic_and_bounded() {
    let raw = "```\n\t/\\_/\\  héllo <&>\n ( o.o ) take 100mg and 2 tabs, 3 ghosts\n  > ^ <\n```\n";
    let a = ascii::sanitize(raw).unwrap();
    assert_eq!(a, ascii::sanitize(raw).unwrap());
    assert_eq!(a.rows.len(), ascii::ROWS);
    assert!(a.rows.iter().all(|r| r.len() == ascii::COLS && r.bytes().all(|c| (0x20..=0x7e).contains(&c))));
    assert_eq!(a.masked_dose_tokens, 2);
    assert!(a.rows[1].contains("take ~~~~~ and ~~~~~~, 3 ghosts"), "{:?}", a.rows[1]);
    assert_eq!(a.replaced_chars, 1);
    assert!(ascii::sanitize("   \n \n").is_err());
    assert!(ascii::sanitize(&format!("{}\n", "#".repeat(200)).repeat(100)).is_err());
}

#[test]
fn cid_matches_known_vectors() {
    // Empty raw block: the well-known CIDv1 of zero bytes.
    assert_eq!(cid::cid_v1_raw(b"").unwrap(), "bafkreihdwdcefgh4dqkjv67uzcmw7ojee6xedzdetojuzjevtenxquvyku");
    assert!(cid::cid_v1_raw(&vec![0u8; cid::MAX_SINGLE_BLOCK + 1]).is_err());
    if let Ok(v) = std::env::var("KUBO_VECTORS") {
        // lines: "<cid> <path>" produced by `ipfs add --only-hash --cid-version 1 -Q`
        for line in std::fs::read_to_string(v).unwrap().lines() {
            let (c, p) = line.split_once(' ').unwrap();
            assert_eq!(cid::cid_v1_raw(&std::fs::read(p).unwrap()).unwrap(), c, "{p}");
        }
    }
}

#[test]
fn symbient_link_template_is_safe_and_packs() {
    let f = ascii::sanitize(&" <&> wob ##########\n /|\\ ##########\n  |  ~~~~ ######## \n".repeat(4)).unwrap();
    let cids = vec![cid::cid_v1_raw(b"a").unwrap(), cid::cid_v1_raw(b"b").unwrap()];
    let t = ascii::template(&[f.clone(), f], &cids, "persona day 1", Some("1;.6;1"));
    assert!(t.contains("&lt;&amp;&gt;"));
    let vars = ascii::variables(&cids);
    common::check_and_pack(&t, &vars).unwrap();
    let p = common::token_params(&vars, 0, b"s", 1);
    let svg = common::render(&t, &vars, &p);
    assert!(svg.contains(&format!("href=\"https://{}/ipfs/{}\"", p[1].value, p[0].value)));
    assert!(svg.contains(&format!("ipfs://{}", p[0].value)));
    // a non-allowlisted gateway or a javascript: link fails the same checks the chain runs
    let mut bad = vars.clone();
    bad[1] = cw_svg::VariableDef { name: "gateway".into(), kind: cw_svg::VariableKind::Options(vec!["evil.example".into()]) };
    assert!(common::check_and_pack(&t, &bad).unwrap_err().contains("links"));
    let js = t.replace("https://${gateway}/ipfs/${frame}", "javascript:alert(1)");
    assert!(common::check_and_pack(&js, &vars).unwrap_err().contains("links"));
}

#[test]
fn rich_text_keeps_full_art_but_masks_doses() {
    let wide = format!("{}\n", "=".repeat(120)).repeat(60) + "take 5 mg now\n";
    let (t, masked, _) = ascii::sanitize_rich(&wide);
    assert_eq!(t.lines().count(), 61);
    assert_eq!(t.lines().next().unwrap().len(), 120); // no 48-col clip
    assert_eq!(masked, 1);
    assert!(t.ends_with("take ~~~~ now"));
}
