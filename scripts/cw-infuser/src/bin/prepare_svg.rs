/// Helper binary that prepares a cw721-svg InstantiateMsg JSON from an SVG template file.
///
/// Usage:
///   cargo run -p cw-infuser-scripts --bin prepare_svg -- --svg path/to/template.svg
///
/// The tool:
///   1. Loads the SVG file and scans for ${varname} placeholders
///   2. Walks you through defining each variable (Options or Range)
///   3. Outputs a complete InstantiateMsg JSON ready for on-chain instantiation
///
/// LLM-assisted mode (--llm):
///   Invokes the `claude` CLI to auto-generate variable definitions using the
///   Placeholder Dictionary Specification. Requires `claude` to be in PATH.
///   Falls back to printing the prompt when the CLI is unavailable.
///
/// Dictionary mode (--dict):
///   Loads a pre-generated *.dict.json file (Collection Layer dictionary).
///   Extracts the "vars" field as the variable definitions.
use anyhow::{anyhow, Context, Result};
use clap::Parser;
use cw721_svg::contract::*;
use cw_svg::*;
use serde_json::Value;
use std::collections::BTreeMap;
use std::fs;
use std::io::{self, BufRead, Write};

// ──────────────────────────────────────────────────────────────────────────────
// Embedded Universal Layer spec (condensed for LLM prompting)
// ──────────────────────────────────────────────────────────────────────────────
const PLACEHOLDER_DICT_SPEC: &str = r#"
# CW-SVG Placeholder Dictionary — Universal Layer v1

## Variable Name Grammar
  varname  ::= class ( "-" qualifier )*
  class    ::= semantic-prefix
  qualifier ::= instance | role-suffix
  instance  ::= letter | digit+
  role-suffix ::= "base" | "lo" | "hi" | "fast" | "slow" | label

## Semantic Class → VariableKind Inference
  color-*       → RgbStyled  (palette_groups ranges; shared across group)
  glow-*        → RgbStyled  (same palette as color group)
  hue-*         → RgbStyled  (hue-anchored ranges)
  opacity-*     → Range [0.0, 1.0] prec 2
  alpha-*       → Range [0.0, 1.0] prec 2
  pulse-*       → Range [1.2, 4.5] prec 2   ← unitless; template appends "s"
  breath-*      → Range [1.2, 4.5] prec 2   ← unitless; template appends "s"
  flicker-*     → Range [0.08, 0.50] prec 2 ← unitless; template appends "s"
  strobe-*      → Range [0.08, 0.50] prec 2 ← unitless; template appends "s"
  drift-*       → Range [3.0, 15.0] prec 1  ← unitless; template appends "s"
  float-*       → Range [3.0, 15.0] prec 1  ← unitless; template appends "s"
  rotate-*      → Range [0, 360] prec 0
  spin-*        → Range [0, 360] prec 0
  scale-*       → Range [0.5, 2.0] prec 2
  size-*        → Range [0.5, 2.0] prec 2
  blur-*        → Range [0.5, 8.0] prec 1
  toggle-*      → Options ["0", "1"]
  mode-*        → Options (infer from SVG context)
  val-*         → Range (infer from SVG context)

## VariableDef JSON format (Vec<VariableDef>)
  // RGB random (full spectrum)
  { "name": "bg", "kind": "rgb" }

  // RgbStyled — constrained palette
  { "name": "color-a", "kind": { "rgb_styled": [
      { "r_min": 50, "r_max": 140, "g_min": 220, "g_max": 255, "b_min": 50, "b_max": 140 }
  ]}}

  // Range — decimal number
  { "name": "pulse-a", "kind": { "range": { "min": "1.2", "max": "4.5", "precision": 2 } } }

  // Options — pick from list
  { "name": "mode-a", "kind": { "options": ["circle", "square"] } }

## Palette coherence rule
  All variables in the same palette group share identical RgbRange entries.

## Timing unit rule
  Timing variables use unitless Range values. The SVG template appends the unit:
    style="--pulse-dur:${pulse-a}s"   ← correct
    style="--pulse-dur:${pulse-a}"    ← incorrect (placeholder would need "2.42s")

## Output format
  Return ONLY a JSON array (Vec<VariableDef>). No prose, no code fences.
  Every variable in the template must have exactly one entry.
"#;

// ──────────────────────────────────────────────────────────────────────────────
// SVG sanitization
// ──────────────────────────────────────────────────────────────────────────────

/// Strip `<!-- ... -->` SVG/XML comments from a template.
///
/// Comments are removed before placeholder scanning so that documentation
/// examples using `${...}` syntax inside comments are not treated as real
/// placeholders, and so that comment bytes don't contribute to on-chain storage.
fn strip_svg_comments(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(open) = rest.find("<!--") {
        result.push_str(&rest[..open]);
        match rest[open..].find("-->") {
            Some(close_rel) => rest = &rest[open + close_rel + 3..],
            None => {
                // Unclosed comment — keep remainder verbatim
                result.push_str(&rest[open..]);
                return result;
            }
        }
    }
    result.push_str(rest);
    result
}

/// Remove `<![CDATA[` and `]]>` markers from a template.
///
/// `<![CDATA[...]]>` inside `<style>` is valid XML but breaks HTML5 inline SVG
/// parsing. Stripping the markers leaves the style content intact (CSS inside
/// CDATA never needs entity escaping in practice) and makes the SVG safe to
/// embed directly in an HTML document or JSON payload.
fn strip_cdata_markers(s: &str) -> String {
    s.replace("<![CDATA[", "").replace("]]>", "")
}

/// Apply both sanitizations and report what changed.
fn sanitize_svg(template: &str) -> (String, bool) {
    let after_comments = strip_svg_comments(template);
    let after_cdata = strip_cdata_markers(&after_comments);
    let changed = after_cdata.len() != template.len();
    (after_cdata, changed)
}

// ──────────────────────────────────────────────────────────────────────────────
// CLI args
// ──────────────────────────────────────────────────────────────────────────────
#[derive(Parser, Debug)]
#[command(
    version,
    about = "Prepare a cw721-svg InstantiateMsg JSON from an SVG template"
)]
struct Args {
    /// Path to the SVG template file containing ${varname} placeholders
    #[arg(long)]
    svg: String,

    /// Collection name
    #[arg(long)]
    name: Option<String>,

    /// Collection symbol
    #[arg(long)]
    symbol: Option<String>,

    /// Total supply
    #[arg(long)]
    total: Option<u64>,

    /// Seed string (will be blake3-hashed). If omitted, prompted interactively.
    #[arg(long)]
    seed: Option<String>,

    /// Path to a JSON file containing raw variable definitions (Vec<VariableDef>).
    /// Format: [{"name":"color_yin","kind":{"options":["red","blue"]}}]
    #[arg(long)]
    vars_json: Option<String>,

    /// Path to a Collection Layer dictionary file (*.dict.json).
    /// Extracts the "vars" field as variable definitions.
    #[arg(long)]
    dict: Option<String>,

    /// Use the `claude` CLI to auto-generate variable definitions from the
    /// Placeholder Dictionary Specification. Requires `claude` in PATH.
    /// If the CLI is not available, prints the prompt for manual use instead.
    #[arg(long, default_value_t = false)]
    llm: bool,

    /// Skip SVG sanitization (stripping comments and CDATA markers).
    /// By default, sanitization runs before placeholder scanning to ensure
    /// the stored template is HTML5-compatible.
    #[arg(long, default_value_t = false)]
    no_sanitize: bool,

    /// Price tiers as a JSON array.
    /// Format: '[{"until_count":100,"price":{"denom":"uterp","amount":"1000000"}}]'
    /// Tiers are applied in order while mint_count < until_count. Empty = free mint.
    #[arg(long)]
    price_tiers: Option<String>,

    /// Output file (defaults to stdout)
    #[arg(long, short)]
    output: Option<String>,
}

// ──────────────────────────────────────────────────────────────────────────────
// Placeholder scanning
// ──────────────────────────────────────────────────────────────────────────────

/// Discovered placeholder in the template
struct Placeholder {
    name: String,
    start: usize,
    end: usize,
}

/// Scan an SVG template for ${varname} placeholders.
/// Returns unique variable names (in order of first appearance) and all slot positions.
fn scan_placeholders(template: &str) -> Result<(Vec<String>, Vec<Placeholder>)> {
    let bytes = template.as_bytes();
    let len = bytes.len();
    let mut placeholders = Vec::new();
    let mut seen_order: Vec<String> = Vec::new();
    let mut i = 0;

    while i < len.saturating_sub(1) {
        if bytes[i] == b'$' && bytes[i + 1] == b'{' {
            let start = i;
            let name_start = i + 2;
            let mut j = name_start;
            while j < len && bytes[j] != b'}' {
                j += 1;
            }
            if j >= len {
                return Err(anyhow!(
                    "Unclosed placeholder at byte offset {} (near: '{}...')",
                    start,
                    &template[start..std::cmp::min(start + 30, len)]
                ));
            }
            let name = template[name_start..j].to_string();
            let end = j + 1;

            if !seen_order.contains(&name) {
                seen_order.push(name.clone());
            }

            placeholders.push(Placeholder { name, start, end });
            i = end;
        } else {
            i += 1;
        }
    }

    Ok((seen_order, placeholders))
}

// ──────────────────────────────────────────────────────────────────────────────
// Interactive definition
// ──────────────────────────────────────────────────────────────────────────────

fn prompt(msg: &str) -> String {
    print!("{}", msg);
    io::stdout().flush().unwrap();
    let mut line = String::new();
    io::stdin().lock().read_line(&mut line).unwrap();
    line.trim().to_string()
}

fn define_variable(name: &str, occurrence_count: usize) -> Result<VariableDef> {
    println!();
    println!("━━━ Variable: ${{{name}}} ({occurrence_count} occurrence(s)) ━━━");
    println!("  [1] Options      — pick from a list of values");
    println!("  [2] Range        — random decimal in [min, max]");
    println!("  [3] Rgb          — random rgb(R,G,B) color");
    println!("  [4] Rgb Styled   — random shade within defined color ranges");
    let choice = prompt("  Type (1, 2, 3, or 4): ");

    let kind = match choice.as_str() {
        "1" => {
            println!("  Enter options one per line. Empty line to finish:");
            let mut opts = Vec::new();
            loop {
                let val = prompt("    > ");
                if val.is_empty() {
                    break;
                }
                opts.push(val);
            }
            if opts.is_empty() {
                return Err(anyhow!("Options list must not be empty for '{}'", name));
            }
            VariableKind::Options(opts)
        }
        "2" => {
            let min = prompt("  min: ");
            let max = prompt("  max: ");
            let prec = prompt("  precision (decimal places, default 0): ");
            let precision = if prec.is_empty() {
                0
            } else {
                prec.parse().context("invalid precision")?
            };
            VariableKind::Range {
                min,
                max,
                precision,
            }
        }
        "3" => VariableKind::Rgb,
        "4" => {
            println!("  Define color ranges. Each range constrains R, G, B channels.");
            println!("  Enter ranges one at a time. Empty r_min to finish.");
            let mut ranges = Vec::new();
            loop {
                println!("  --- Range {} ---", ranges.len() + 1);
                let r_min = prompt("    r_min (0-255, empty to finish): ");
                if r_min.is_empty() {
                    break;
                }
                let r_max = prompt("    r_max (0-255): ");
                let g_min = prompt("    g_min (0-255): ");
                let g_max = prompt("    g_max (0-255): ");
                let b_min = prompt("    b_min (0-255): ");
                let b_max = prompt("    b_max (0-255): ");
                ranges.push(RgbRange {
                    r_min: r_min.parse().context("invalid r_min")?,
                    r_max: r_max.parse().context("invalid r_max")?,
                    g_min: g_min.parse().context("invalid g_min")?,
                    g_max: g_max.parse().context("invalid g_max")?,
                    b_min: b_min.parse().context("invalid b_min")?,
                    b_max: b_max.parse().context("invalid b_max")?,
                });
            }
            if ranges.is_empty() {
                return Err(anyhow!(
                    "Rgb Styled requires at least one range for '{}'",
                    name
                ));
            }
            VariableKind::RgbStyled(ranges)
        }
        _ => {
            return Err(anyhow!(
                "Invalid choice '{}', expected 1, 2, 3, or 4",
                choice
            ))
        }
    };

    Ok(VariableDef {
        name: name.to_string(),
        kind,
    })
}

// ──────────────────────────────────────────────────────────────────────────────
// LLM-assisted generation
// ──────────────────────────────────────────────────────────────────────────────

/// Build the prompt sent to the LLM.
fn build_llm_prompt(
    svg_path: &str,
    template: &str,
    var_names: &[String],
    counts: &BTreeMap<&str, usize>,
) -> String {
    let vars_list: String = var_names
        .iter()
        .map(|n| {
            let c = counts.get(n.as_str()).copied().unwrap_or(0);
            format!("  - ${{{n}}} ({c} occurrence(s))")
        })
        .collect::<Vec<_>>()
        .join("\n");

    // Include a preview of the SVG (first 3000 chars) for context.
    let preview_len = template.len().min(3000);
    let preview = &template[..preview_len];
    let truncated = if template.len() > preview_len {
        "\n... (truncated)"
    } else {
        ""
    };

    format!(
        "{spec}\n\
         ---\n\
         SVG file: {svg_path}\n\
         Variables found:\n\
         {vars_list}\n\n\
         SVG template preview:\n\
         {preview}{truncated}\n\
         ---\n\
         Output a JSON array (Vec<VariableDef>) with one entry per variable above.\n\
         No prose. No code fences. Pure JSON array only.",
        spec = PLACEHOLDER_DICT_SPEC,
        svg_path = svg_path,
        vars_list = vars_list,
        preview = preview,
        truncated = truncated,
    )
}

/// Try to call the `claude` CLI with the given prompt.
/// Returns the response text on success, or an error if the CLI is unavailable/fails.
fn call_claude_cli(prompt_text: &str) -> Result<String> {
    let output = std::process::Command::new("claude")
        .arg("-p")
        .arg(prompt_text)
        .output()
        .context("`claude` CLI not found — install it or use --vars-json / --dict instead")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow!("claude CLI exited with error:\n{}", stderr));
    }

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

/// Extract the first JSON array `[...]` from an LLM response string.
fn extract_json_array(response: &str) -> Result<&str> {
    let start = response
        .find('[')
        .ok_or_else(|| anyhow!("No JSON array found in LLM response"))?;
    let end = response
        .rfind(']')
        .ok_or_else(|| anyhow!("No closing ']' found in LLM response"))?;
    if end <= start {
        return Err(anyhow!("Malformed JSON array in LLM response"));
    }
    Ok(&response[start..=end])
}

/// Run LLM-assisted variable definition.
/// Calls `claude` CLI if available; otherwise prints the prompt for manual use.
fn define_variables_with_llm(
    svg_path: &str,
    template: &str,
    var_names: &[String],
    counts: &BTreeMap<&str, usize>,
) -> Result<Vec<VariableDef>> {
    let prompt_text = build_llm_prompt(svg_path, template, var_names, counts);

    println!("\nCalling `claude` CLI to generate variable definitions...");

    let response = match call_claude_cli(&prompt_text) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("\nWarning: {e}");
            eprintln!("\nCould not invoke the claude CLI automatically.");
            eprintln!("Paste the following prompt into your LLM, then save the JSON");
            eprintln!("output to a file and re-run with --vars-json <file>.\n");
            eprintln!("──────── PROMPT START ────────");
            eprintln!("{prompt_text}");
            eprintln!("──────── PROMPT END ──────────");
            return Err(anyhow!("LLM call failed — see prompt above"));
        }
    };

    println!(
        "LLM response received ({} chars). Parsing...",
        response.len()
    );

    let json_str = extract_json_array(&response).with_context(|| {
        format!(
            "Raw LLM response:\n{}",
            &response[..response.len().min(500)]
        )
    })?;

    let vars: Vec<VariableDef> = serde_json::from_str(json_str)
        .with_context(|| format!("Failed to parse LLM output as Vec<VariableDef>:\n{json_str}"))?;

    // Show user what was generated and ask for confirmation.
    println!("\nGenerated {} variable definition(s):", vars.len());
    for v in &vars {
        let kind_label = match &v.kind {
            VariableKind::Rgb => "Rgb".to_string(),
            VariableKind::RgbStyled(r) => format!("RgbStyled ({} range(s))", r.len()),
            VariableKind::Range {
                min,
                max,
                precision,
            } => {
                format!("Range [{min}, {max}] prec {precision}")
            }
            VariableKind::Options(o) => format!("Options ({} values)", o.len()),
        };
        println!("  ${{{}}}: {}", v.name, kind_label);
    }

    let confirm = prompt("\nAccept these definitions? [y/N]: ");
    if confirm.to_lowercase() != "y" {
        return Err(anyhow!(
            "Definitions rejected. Re-run with --vars-json to provide your own."
        ));
    }

    Ok(vars)
}

// ──────────────────────────────────────────────────────────────────────────────
// Dictionary file loading
// ──────────────────────────────────────────────────────────────────────────────

/// Load a Collection Layer dictionary file (*.dict.json) and extract the "vars" field.
fn load_dict_file(path: &str, var_names: &[String]) -> Result<Vec<VariableDef>> {
    let raw = fs::read_to_string(path)
        .with_context(|| format!("Failed to read dictionary file: {path}"))?;

    let doc: Value = serde_json::from_str(&raw)
        .with_context(|| format!("Failed to parse dictionary JSON from {path}"))?;

    // Accept either a top-level array (raw vars JSON) or a dict object with a "vars" key.
    let vars_value = if doc.is_array() {
        doc
    } else {
        doc.get("vars")
            .ok_or_else(|| {
                anyhow!(
                    "Dictionary file {path} has no 'vars' field. \
                     Expected either a JSON array or an object with a 'vars' array."
                )
            })?
            .clone()
    };

    let vars: Vec<VariableDef> = serde_json::from_value(vars_value)
        .with_context(|| format!("Failed to parse 'vars' as Vec<VariableDef> from {path}"))?;

    // Verify all template variables are covered.
    for name in var_names {
        if !vars.iter().any(|v| v.name == *name) {
            return Err(anyhow!(
                "Template variable '{}' not found in dictionary '{}'",
                name,
                path
            ));
        }
    }

    println!(
        "Loaded {} variable definition(s) from dictionary: {}",
        vars.len(),
        path
    );
    Ok(vars)
}

// ──────────────────────────────────────────────────────────────────────────────
// Main
// ──────────────────────────────────────────────────────────────────────────────

pub fn main() -> Result<()> {
    let args = Args::parse();

    // 1. Load SVG template
    let raw_template =
        fs::read_to_string(&args.svg).with_context(|| format!("Failed to read: {}", args.svg))?;

    println!("Loaded SVG template: {} bytes", raw_template.len());

    // 1b. Sanitize — strip comments and CDATA markers before scanning so that
    //     doc examples in comments aren't treated as placeholders, and the
    //     stored template is HTML5-compatible (no CDATA inside <style>).
    let template = if args.no_sanitize {
        raw_template
    } else {
        let (sanitized, changed) = sanitize_svg(&raw_template);
        if changed {
            println!(
                "Sanitized SVG: {} bytes (stripped comments / CDATA markers)",
                sanitized.len()
            );
        }
        sanitized
    };

    // 2. Scan for placeholders
    let (var_names, placeholders) = scan_placeholders(&template)?;

    if var_names.is_empty() {
        println!("No ${{...}} placeholders found in template.");
        println!("The SVG will be stored as-is with no variable substitution.");
    } else {
        println!("Found {} unique variable(s):", var_names.len());
        let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
        for ph in &placeholders {
            *counts.entry(&ph.name).or_default() += 1;
        }
        for name in &var_names {
            let count = counts.get(name.as_str()).unwrap_or(&0);
            println!("  ${{{name}}}  — {count} occurrence(s)");
        }
    }

    // 3. Define variables — four mutually exclusive paths, in priority order:
    //    1. --vars-json  raw VariableDef array
    //    2. --dict       Collection Layer dictionary file
    //    3. --llm        LLM-assisted via claude CLI
    //    4. interactive  manual per-variable prompts
    let variables: Vec<VariableDef> = if let Some(vars_path) = &args.vars_json {
        let vars_str = fs::read_to_string(vars_path)
            .with_context(|| format!("Failed to read vars JSON: {}", vars_path))?;
        let vars: Vec<VariableDef> = serde_json::from_str(&vars_str)
            .with_context(|| format!("Failed to parse vars JSON from {}", vars_path))?;
        for name in &var_names {
            if !vars.iter().any(|v| v.name == *name) {
                return Err(anyhow!(
                    "Template variable '{}' not found in vars JSON",
                    name
                ));
            }
        }
        println!(
            "Loaded {} variable definition(s) from {}",
            vars.len(),
            vars_path
        );
        vars
    } else if let Some(dict_path) = &args.dict {
        load_dict_file(dict_path, &var_names)?
    } else if args.llm {
        let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
        for ph in &placeholders {
            *counts.entry(&ph.name).or_default() += 1;
        }
        define_variables_with_llm(&args.svg, &template, &var_names, &counts)?
    } else {
        let mut vars = Vec::new();
        let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
        for ph in &placeholders {
            *counts.entry(&ph.name).or_default() += 1;
        }
        for name in &var_names {
            let count = *counts.get(name.as_str()).unwrap_or(&0);
            let var_def = define_variable(name, count)?;
            vars.push(var_def);
        }
        vars
    };

    // 4. Build template_slots
    // var_idx must index into the on-chain `variables` array, so derive the
    // mapping from `variables` order, not from template first-appearance order.
    let var_index: BTreeMap<&str, u16> = variables
        .iter()
        .enumerate()
        .map(|(i, v)| (v.name.as_str(), i as u16))
        .collect();

    let template_slots: Vec<TemplateSlot> = placeholders
        .iter()
        .map(|ph| TemplateSlot {
            start: ph.start as u32,
            end: ph.end as u32,
            var_idx: var_index[ph.name.as_str()],
        })
        .collect();

    // 5. Collect remaining fields
    let name = args.name.unwrap_or_else(|| prompt("Collection name: "));
    let symbol = args.symbol.unwrap_or_else(|| prompt("Collection symbol: "));
    let total = match args.total {
        Some(t) => t,
        None => {
            let t = prompt("Total supply: ");
            t.parse().context("invalid total supply")?
        }
    };

    let seed_input = args
        .seed
        .unwrap_or_else(|| prompt("Seed (any string, will be hashed): "));
    let seed_bytes = blake3::hash(seed_input.as_bytes());
    let seed = cosmwasm_std::Binary::from(seed_bytes.as_bytes().as_slice());

    // 6. Parse price tiers
    let price_tiers: Vec<PriceTier> = match args.price_tiers {
        Some(ref json) => serde_json::from_str(json).context(
            "--price-tiers must be a JSON array of {until_count, price: {denom, amount}}",
        )?,
        None => vec![],
    };

    // 7. Pre-flight validation — mirror the on-chain instantiate checks so
    //    errors are caught here rather than burning gas on-chain.
    println!("\nRunning pre-flight validation...");
    if total > MAX_TOTAL_SUPPLY {
        return Err(anyhow!(
            "total supply {} exceeds on-chain limit of {}",
            total,
            MAX_TOTAL_SUPPLY
        ));
    }
    validate_variables(&variables).map_err(|e| anyhow!("Variable definition error: {}", e))?;
    validate_template_slots(&template, &variables, &template_slots)
        .map_err(|e| anyhow!("Template slot error: {}", e))?;
    let packed = cw721_svg::pack_svg(&template, &variables);
    if packed.len() > MAX_SVG_SIZE {
        return Err(anyhow!(
            "packed template is {} bytes — exceeds on-chain limit of {} bytes",
            packed.len(),
            MAX_SVG_SIZE
        ));
    }
    println!(
        "  ✓ template {} bytes, packed {} bytes  (limit {})",
        template.len(),
        packed.len(),
        MAX_SVG_SIZE
    );
    println!("  ✓ {} variable(s) valid", variables.len());
    println!("  ✓ {} template slot(s) valid", template_slots.len());

    // 7b. Build InstantiateMsg
    let msg = cw721_svg::InstantiateMsg {
        name,
        symbol,
        collection_info_extension: cw721_svg::SvgCollectionMetadata {
            seed,
            packed,
            price_tiers,
            total,
            mint_start_time: None,
            mint_end_time: None,
            whitelist: None,
            require_auth: false,
        },
        minter: None,
        creator: None,
        withdraw_address: None,
    };

    // 8. Output JSON
    let json = serde_json::to_string_pretty(&msg)?;

    match args.output {
        Some(path) => {
            fs::write(&path, &json).with_context(|| format!("Failed to write: {}", path))?;
            println!("\nInstantiateMsg written to: {path}");
        }
        None => {
            println!("\n{json}");
        }
    }

    Ok(())
}
