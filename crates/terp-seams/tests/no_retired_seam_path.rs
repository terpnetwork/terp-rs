//! CI: no Cargo.toml may depend on deleted shim `seam_private_dex` / package
//! `private_dex_seams`. Product crate is `terp-seams` only.

use std::fs;
use std::path::Path;

fn walk_tomls(dir: &Path, out: &mut Vec<(String, String)>) {
    let rd = match fs::read_dir(dir) {
        Ok(r) => r,
        Err(_) => return,
    };
    for ent in rd.flatten() {
        let p = ent.path();
        let name = ent.file_name();
        if name == "target" || name == ".git" || name == "node_modules" {
            continue;
        }
        if p.is_dir() {
            walk_tomls(&p, out);
        } else if name == "Cargo.toml" {
            if let Ok(s) = fs::read_to_string(&p) {
                out.push((p.display().to_string(), s));
            }
        }
    }
}

#[test]
fn no_private_dex_seams_package_or_seam_private_dex_path() {
    let terp_rs = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut tomls = Vec::new();
    walk_tomls(&terp_rs, &mut tomls);
    let pir = terp_rs.join("../private-inference-rent");
    if pir.is_dir() {
        walk_tomls(&pir, &mut tomls);
    }

    let mut offenders = Vec::new();
    for (path, body) in &tomls {
        if path.contains("/target/") {
            continue;
        }
        for line in body.lines() {
            let t = line.trim();
            if t.starts_with('#') {
                continue;
            }
            if t.contains("seam_private_dex") || t.contains("private_dex_seams") {
                offenders.push(format!("{path}: {t}"));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "use terp-seams only (feature dex); do not depend on deleted shim:\n{}",
        offenders.join("\n")
    );
}
