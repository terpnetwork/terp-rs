//! WASM module preflight: check and build wasm-pack crates needed by the frontend.
//!
//! Each page that uses client-side WASM declares its module here. The preflight
//! verifies the compiled `.wasm` + `.js` glue exist in `pkg/`, and builds any
//! that are missing via `wasm-pack build --target web`.

use std::path::{Path, PathBuf};
use std::process::Command;

/// Website `pkg/` directory where all WASM outputs live.
fn pkg_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../pkg")
}

/// A WASM module required by the frontend.
struct WasmModule {
    /// Module name (e.g. `norick_wasm`). Files are `{name}.js` + `{name}_bg.wasm`.
    name: &'static str,
    /// Path to the Rust crate (absolute or relative to CARGO_MANIFEST_DIR).
    crate_path: Option<PathBuf>,
    /// Which page uses this module.
    page: &'static str,
}

/// Registry of all WASM modules the website needs.
fn wasm_modules() -> Vec<WasmModule> {
    let terp_rs = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../terp-rs");

    vec![WasmModule {
        name: "norick_wasm",
        crate_path: Some(terp_rs.join("crates/wasm-bindgen/norick-wasm")),
        page: "no-rick.html",
    }]
}

/// Check if a module's compiled outputs exist in `pkg/`.
fn module_ready(name: &str) -> bool {
    let pkg = pkg_dir();
    pkg.join(format!("{name}.js")).exists() && pkg.join(format!("{name}_bg.wasm")).exists()
}

/// Build a WASM module with `wasm-pack build --target web`.
///
/// Outputs go to a temp dir, then the needed files are copied into `pkg/`.
fn build_module(name: &str, crate_path: &Path) -> anyhow::Result<()> {
    println!("Building WASM module: {} from {:?}", name, crate_path);

    if !crate_path.join("Cargo.toml").exists() {
        anyhow::bail!(
            "Crate not found at {:?} — cannot build {}",
            crate_path,
            name
        );
    }

    // Check wasm-pack is available
    let check = Command::new("wasm-pack").arg("--version").output();
    if check.is_err() || !check.unwrap().status.success() {
        anyhow::bail!("wasm-pack not found. Install: cargo install wasm-pack");
    }

    // Build to a temp directory to avoid overwriting pkg/package.json
    let tmp_out = std::env::temp_dir().join(format!("wasm-build-{name}"));
    let _ = std::fs::remove_dir_all(&tmp_out);

    let status = Command::new("wasm-pack")
        .args([
            "build",
            "--target",
            "web",
            "--release",
            "--out-dir",
            &tmp_out.to_string_lossy(),
        ])
        .current_dir(crate_path)
        .status()?;

    if !status.success() {
        anyhow::bail!("wasm-pack build failed for {}", name);
    }

    // Copy outputs into pkg/
    let pkg = pkg_dir();
    std::fs::create_dir_all(&pkg)?;

    let files_to_copy = [
        format!("{name}.js"),
        format!("{name}_bg.wasm"),
        format!("{name}.d.ts"),
        format!("{name}_bg.wasm.d.ts"),
    ];

    for file in &files_to_copy {
        let src = tmp_out.join(file);
        if src.exists() {
            std::fs::copy(&src, pkg.join(file))?;
        }
    }

    // Cleanup temp
    let _ = std::fs::remove_dir_all(&tmp_out);

    println!("  -> {} ready in pkg/", name);
    Ok(())
}

/// Run the WASM preflight: check all modules, build any that are missing.
///
/// Returns a list of modules that are still missing after attempting builds.
pub fn preflight() -> Vec<String> {
    let modules = wasm_modules();
    let mut still_missing = Vec::new();

    for m in &modules {
        if module_ready(m.name) {
            println!("  WASM ok: {} ({})", m.name, m.page);
            continue;
        }

        // Try to build if we have a crate path
        if let Some(ref crate_path) = m.crate_path {
            match build_module(m.name, crate_path) {
                Ok(()) => {
                    if module_ready(m.name) {
                        continue;
                    }
                }
                Err(e) => {
                    eprintln!("  WASM build failed for {}: {}", m.name, e);
                }
            }
        }

        // Still missing
        eprintln!(
            "  WASM missing: {} (needed by {}){}",
            m.name,
            m.page,
            if m.crate_path.is_some() {
                " — build failed"
            } else {
                " — no crate path, rebuild manually"
            }
        );
        still_missing.push(format!("{} ({})", m.name, m.page));
    }

    still_missing
}

/// Check without building — just report what's present and what's missing.
pub fn status() {
    let modules = wasm_modules();
    let pkg = pkg_dir();
    println!("WASM modules (pkg dir: {:?}):", pkg);
    for m in &modules {
        let ready = module_ready(m.name);
        println!(
            "  {} {} — {}",
            if ready { "ok" } else { "MISSING" },
            m.name,
            m.page,
        );
    }
}
