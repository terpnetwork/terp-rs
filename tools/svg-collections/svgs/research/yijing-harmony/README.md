# Yi Jing Harmony (dao-harmony 10-19-26): cw721-svg inputs

These are the inputs for `src/yijing.rs` and `src/bin/yijing_harmony.rs`. The target contract is the packed-template cw721-svg (SVP1, stored raw under `svg_packed`).

| File | What it is |
|---|---|
| `template.svg` | The prototype template, byte for byte (10,376 B). It has 133 `${var}` names and 165 slots. |
| `variables.json` | `Vec<cw_svg::VariableDef>` for the **current** contract. |
| `v2-variables.json` | The same names and order using the proposed `SuppliedOptions` / `SuppliedText` kinds, for the v2 per-token-values handoff (spec §v2 handoff). |

How the variables map onto the current contract:
- **121 option variables** map 1:1 to `Options`. Their lists are identical to the v2 `SuppliedOptions` lists, so in v2 they only change kind.
- **12 text variables** map to `Options` over the 4 sample texts. They're for previews only and carry `TODO(yijing-v2-values)`.

Today the contract picks every variable from sha256(token_id‖seed‖height). The art is valid SVG, but the hexagram shown isn't the tournament's result until v2.

Placeholders, each with a single swap point (CLI flag or env var; see `yijing_harmony --help`):

| Placeholder | Default | Swap point |
|---|---|---|
| Price denom | `ibc/<PENUMBRA_USDT_TBD>` | `--denom` / `YIJING_DENOM` |
| Decimals | 6, TBD | `--usdt-decimals` / `YIJING_USDT_DECIMALS` |
| Minter | `<MINTER_ADDR_TBD>` | `--minter` |
| Creator | `<CREATOR_ADDR_TBD>` | `--creator` |
| Withdraw address | `<WITHDRAW_ADDR_TBD>` | `--withdraw` |
| Royalty recipient | `<ROYALTY_RECIPIENT_TBD>` | `--royalty-recipient` |

The 9% royalty isn't part of cw721-svg. It goes to the marketplace as `royalty.json` (`set_royalty_fee`).
