---
title: CW721-SVG
description: Fully on-chain SVG NFTs — template placeholders, variable kinds, pricing, and prepare_svg scripting.
---

# CW721-SVG

Fully on-chain SVG NFTs. The contract stores an SVG template with `${varname}` placeholders that get substituted with deterministically randomized values at mint time. Each token's metadata is a set of resolved parameters — no off-chain storage, no IPFS, no external dependencies.

Templates are capped at 420KB. Total supply is capped at 10,000.

## Available Choices

Variables define what gets substituted into the template. Each `${varname}` placeholder in the SVG maps to a `VariableDef` with one of the following kinds:

### RGB - Random

`VariableKind::Rgb` — generates a random `rgb(R,G,B)` color string where each channel is independently and uniformly random across all 256 values (0–255). Entropy is derived from `sha256(token_seed || var_idx)`, so each variable gets its own independent color.

Use this when you want full-spectrum randomness with no constraints on the resulting color.

```json
{ "name": "bg_color", "kind": "rgb" }
```

In the SVG template: `fill="${bg_color}"` → resolves to e.g. `fill="rgb(42,187,3)"`.

### RGB - Stylized Random

`VariableKind::RgbStyled(Vec<RgbRange>)` — constrained RGB generation. Creators define a set of color ranges, each with per-channel min/max bounds. At mint time, the contract picks a range uniformly at random and then generates a random shade within that range's constraints.

This gives artistic control over color palettes while preserving on-chain randomness. For example, a creator could restrict a variable to warm tones by defining ranges that only cover reds/oranges/yellows. Each mint produces a unique shade, but always within the intended palette.

```json
{
  "name": "accent",
  "kind": {
    "rgb_styled": [
      { "r_min": 180, "r_max": 255, "g_min": 0, "g_max": 80, "b_min": 0, "b_max": 60 },
      { "r_min": 0, "r_max": 60, "g_min": 50, "g_max": 150, "b_min": 180, "b_max": 255 }
    ]
  }
}
```

Each `RgbRange` has `r_min`, `r_max`, `g_min`, `g_max`, `b_min`, `b_max` (all `u8`, 0–255). Validation enforces min <= max per channel and at least one range defined.

### Variable Range

`VariableKind::Range { min, max, precision }` — generates a random decimal value between `min` and `max` (inclusive) at the specified decimal precision.

`min` and `max` are decimal strings (e.g. `"-2.5"`, `"80"`, `"0.45"`). Precision controls decimal places: precision `0` produces integers, precision `2` produces values like `"12.34"`.

```json
{ "name": "rotation", "kind": { "range": { "min": "0", "max": "360", "precision": 0 } } }
{ "name": "opacity", "kind": { "range": { "min": "0.1", "max": "1.0", "precision": 2 } } }
```

In the SVG template: `transform="rotate(${rotation})"` → resolves to e.g. `transform="rotate(217)"`.

### Variable Set

`VariableKind::Options(Vec<String>)` — picks uniformly at random from a predefined list of string values. The options list must not be empty.

```json
{ "name": "shape", "kind": { "options": ["circle", "square", "triangle"] } }
{ "name": "palette", "kind": { "options": ["#FF0000", "#00FF00", "#0000FF"] } }
```

In the SVG template: `<use href="#${shape}"/>` → resolves to e.g. `<use href="#circle"/>`.

## Template Slots

Template slots are pre-computed byte offsets that map each `${varname}` occurrence in the SVG template to its position and variable index. This enables single-pass O(n) substitution at query time instead of scanning the entire template per variable.

Slots are provided by the creator in the `InstantiateMsg` and validated on-chain. Each slot contains:
- `start` — byte offset of `$` in the template
- `end` — byte offset of the byte after `}` in the template
- `var_idx` — index into the variables array

Use the `prepare_svg` script (see below) to compute slots automatically from your SVG file.

## Front End Integrations

The contract exposes two SVG query endpoints:

- **`SvgTokenUri { token_id }`** — returns the fully rendered SVG for a minted token with all placeholders resolved to the token's stored parameters.
- **`SvgPlaceholder { seed }`** — returns a preview SVG with random values filled in, without minting. Pass different `seed` strings to get different previews. Useful for collection previews and mint page UI.

Both return `SvgTokenUriResponse { svg: String }` where `svg` is the complete SVG markup ready for rendering.

## Minting Parameters

- **`seed`** — base entropy for randomization, combined with token ID and block height to produce unique per-token values
- **`total`** — maximum supply
- **`mint_start_time`** / **`mint_end_time`** — optional time windows (specified as seconds from instantiation)
- **`paused`** — owner can pause/unpause minting
- **`payment_address`** — address that receives mint payments (defaults to owner)
- **`whitelist`** — optional merkle whitelist contract address; whitelisted minters bypass fees

### Tiered Pricing

Price tiers define escalating mint costs as supply increases. Tiers are sorted ascending by `until_count`:

```json
"price_tiers": [
  { "until_count": 100, "price": { "denom": "ustars", "amount": "1000000" } },
  { "until_count": 500, "price": { "denom": "ustars", "amount": "5000000" } }
]
```

This means: first 100 mints cost 1 STARS, mints 101–500 cost 5 STARS. An empty `price_tiers` list means free minting.

When minting multiple tokens in a single transaction, costs are calculated per-token based on the running count, so a batch mint that crosses a tier boundary pays the correct price for each token.

## Placeholder Dictionary

Variable names in `${...}` placeholders follow a structured naming convention that encodes semantic intent. This allows tooling and LLMs to automatically infer appropriate `VariableKind` configurations without per-variable manual input.

See [Placeholder dictionary](./dictionary) for the full **Leaning Tower of Babel** specification — a stable Universal Layer grammar with per-collection dictionary files.

Each SVG can ship a companion `*.dict.json` (Collection Layer) that describes its palette groups, motion groups, and concrete `VariableDef` entries.

Quick name → kind inference:

| Prefix | Kind | Notes |
|---|---|---|
| `color-*`, `glow-*` | `RgbStyled` | shared palette ranges |
| `pulse-*`, `breath-*` | `Range` [1.2, 4.5] | unitless — template appends `s` |
| `flicker-*`, `strobe-*` | `Range` [0.08, 0.50] | unitless — template appends `s` |
| `opacity-*`, `alpha-*` | `Range` [0.0, 1.0] | |
| `rotate-*`, `spin-*` | `Range` [0, 360] | |
| `toggle-*` | `Options` | `["0","1"]` |

### Timing unit convention

Timing variables produce unitless decimals. The SVG template appends the CSS unit:

```xml
<!-- correct -->
style="--pulse-dur:${pulse-a}s"

<!-- incorrect (Range can't produce "2.42s") -->
style="--pulse-dur:${pulse-a}"
```

## Scripting

Use the `prepare_svg` binary to build an `InstantiateMsg` JSON from an SVG template file.

### Interactive (default)

```sh
cargo run -p cw-infuser-scripts --bin prepare_svg -- \
  --svg path/to/template.svg \
  --name "My Collection" \
  --symbol "SVG" \
  --total 1000 \
  --seed "my-secret-seed"
```

### Dictionary file (`--dict`)

Pass a `*.dict.json` Collection Layer file to skip interactive prompts:

```sh
cargo run -p cw-infuser-scripts --bin prepare_svg -- \
  --svg packages/cw-svg/svgs/terp-strobe-v2.svg \
  --dict packages/cw-svg/svgs/terp-strobe.dict.json \
  --name "Terp Strobe" --symbol "STROBE" --total 1000 \
  --seed "entropy" -o init-msg.json
```

### LLM-assisted (`--llm`)

Let the `claude` CLI auto-generate variable definitions using the Placeholder Dictionary Specification. Requires `claude` in `PATH`.

```sh
cargo run -p cw-infuser-scripts --bin prepare_svg -- \
  --svg packages/cw-svg/svgs/terp-strobe-v2.svg \
  --llm \
  --name "Terp Strobe" --symbol "STROBE" --total 1000 \
  --seed "entropy" -o init-msg.json
```

If `claude` is not in `PATH`, the tool prints the full prompt for manual pasting into any LLM. The LLM output (a `Vec<VariableDef>` JSON array) can then be saved and passed via `--vars-json`.

### Raw vars JSON (`--vars-json`)

```sh
cargo run -p cw-infuser-scripts --bin prepare_svg -- \
  --svg template.svg \
  --vars-json vars.json \
  --seed "entropy" \
  --name "Collection" \
  --symbol "COL" \
  --total 500 \
  -o init-msg.json
```

The `load_svg_init_msg` function in the scripts library loads a generated JSON file back into the typed `InstantiateMsg` for use in deployment scripts.

## Future Iterations

- time based svg: dynamic rendering based on block height / time progressed from contract mint
- test between packed u64 approach from any optimizations in r/w + seralizations
- mint default svg at a premium
- rotate default svg (if enabled)