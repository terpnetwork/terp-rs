//! svgc marain <out_dir> <seed_hex> [themes.json]   (scribe-slot orb; nothing prepacked)
//! svgc symbient <day_build.json> <out_dir>
//! Writes template.svg, variables.json, packed.svp, build-report.json, example-token-<n>.svg.
use std::{fs, path::Path};
use svg_collections::{ascii, common, marain};

fn hex(s: &str) -> Vec<u8> {
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
}

fn emit(out: &Path, template: &str, vars: &[cw_svg::VariableDef], seed: &[u8], examples: &[u64], label: &str) {
    fs::create_dir_all(out).unwrap();
    let (packed, report) = common::check_and_pack(template, vars).unwrap_or_else(|e| panic!("{e}"));
    fs::write(out.join("template.svg"), template).unwrap();
    fs::write(out.join("variables.json"), serde_json::to_string_pretty(vars).unwrap()).unwrap();
    fs::write(out.join("packed.svp"), &packed).unwrap();
    fs::write(out.join("build-report.json"), serde_json::to_string_pretty(&report).unwrap()).unwrap();
    for (n, tid) in examples.iter().enumerate() {
        let params = common::token_params(vars, *tid, seed, 1000 + *tid);
        let mut svg = common::render(template, vars, &params);
        cw_svg::validate_animated_paint(&svg).expect("rendered paint");
        svg = svg.replacen("<svg ", &format!("<!-- {label}; token_id={tid} simulated height={} -->\n<svg ", 1000 + tid), 1);
        fs::write(out.join(format!("example-token-{}.svg", n + 1)), svg).unwrap();
        fs::write(out.join(format!("example-token-{}.params.json", n + 1)), serde_json::to_string_pretty(&params).unwrap()).unwrap();
    }
    println!("{}", serde_json::to_string_pretty(&report).unwrap());
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    match a.get(1).map(|s| s.as_str()) {
        Some("marain") => {
            // svgc marain <out> <seed_hex> [themes.json]
            let themes = match a.get(4) {
                Some(p) => { let v: serde_json::Value = serde_json::from_str(&fs::read_to_string(p).unwrap()).unwrap();
                    v["themes"].as_array().unwrap().iter().map(|x| x.as_str().unwrap().to_string()).collect() }
                None => marain::themes(),
            };
            emit(Path::new(&a[2]), &marain::template(), &marain::variables(&themes), &hex(&a[3]), &[0, 1, 2],
                 "EXAMPLE (local simulation, not minted)");
        }
        Some("marain-scribe") => {
            // svgc marain-scribe <out_dir> <seed_hex> <token_n> <text>: render example token after a scribe call
            let themes = marain::themes();
            let vars = marain::variables(&themes);
            marain::validate_text(&a[5]).unwrap_or_else(|e| panic!("{e}"));
            let tid: u64 = a[4].parse().unwrap();
            let mut params = common::token_params(&vars, tid, &hex(&a[3]), 1000 + tid);
            let theme = std::mem::replace(&mut params[0].value, a[5].clone());
            let svg = common::render(&marain::template(), &vars, &params);
            cw_svg::validate_animated_paint(&svg).unwrap();
            let svg = svg.replacen("<svg ", &format!("<!-- EXAMPLE (local simulation, not minted); token_id={tid}; theme at mint={theme:?}; scribed text={:?} -->\n<svg ", a[5]), 1);
            fs::write(Path::new(&a[2]).join(format!("example-token-{}-scribed.svg", tid + 1)), svg).unwrap();
        }
        Some("rich-text") => {
            let (t, m, r) = ascii::sanitize_rich(&fs::read_to_string(&a[2]).unwrap());
            println!("{}", serde_json::json!({"text": t, "masked_dose_tokens": m, "replaced_chars": r}));
        }
        Some("cid") => println!("{}", svg_collections::cid::cid_v1_raw(&fs::read(&a[2]).unwrap()).unwrap_or_else(|e| panic!("{e}"))),
        Some("symbient") => {
            #[derive(serde::Deserialize)]
            struct DayBuild { raw_frames: Vec<String>, rich_cids: Vec<String>, caption: String, flicker: Option<String>, seed_hex: String }
            let d: DayBuild = serde_json::from_str(&fs::read_to_string(&a[2]).unwrap()).unwrap();
            let frames: Vec<_> = d.raw_frames.iter().map(|r| ascii::sanitize(r).unwrap_or_else(|e| panic!("{e}"))).collect();
            let out = Path::new(&a[3]);
            fs::create_dir_all(out).unwrap();
            let san: Vec<_> = frames.iter().map(|f| serde_json::json!({
                "sanitized_sha256": common::sha256_hex(f.rows.join("\n").as_bytes()),
                "replaced_chars": f.replaced_chars, "masked_dose_tokens": f.masked_dose_tokens, "ink_permille": f.ink_permille})).collect();
            fs::write(out.join("sanitize-report.json"), serde_json::to_string_pretty(&san).unwrap()).unwrap();
            let t = ascii::template(&frames, &d.rich_cids, &d.caption, d.flicker.as_deref());
            emit(out, &t, &ascii::variables(&d.rich_cids), &hex(&d.seed_hex), &[0, 1, 2],
                 "EXAMPLE (local simulation, not minted)");
        }
        Some("sanitize") => {
            match ascii::sanitize(&fs::read_to_string(&a[2]).unwrap()) {
                Ok(s) => println!("{}", serde_json::to_string(&s).unwrap()),
                Err(e) => { eprintln!("{e}"); std::process::exit(1) }
            }
        }
        _ => { eprintln!("usage: svgc marain <out> <seed_hex> [themes.json] | marain-scribe <out> <seed_hex> <tid> <text> | symbient <day_build.json> <out> | sanitize <raw> | cid <file>"); std::process::exit(2) }
    }
}
