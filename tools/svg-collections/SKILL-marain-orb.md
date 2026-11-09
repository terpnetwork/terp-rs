# SKILL: Marain orb — scribe curation

Each token in a packed-only cw721-svg collection shows a short text in Marain-style 3x3 glyphs on the core of the terp orb shield. The text changes in two steps.

1. **Mint.** The contract picks one theme from the curated list in `data/marain-themes.json` (32 themes) and shows it.
   - The template's `scribe` options variable holds the theme list.
   - The pick comes from the token seed: `sha256(token_id ‖ collection seed ‖ block height)`, then `sha256(seed ‖ var_idx)` gives a u64, and the index is that u64 mod 32. 32 divides 2^64, so there is no modulo bias.
2. **Scribe.** An allowlisted scribe sends `Scribe { token_id, text }` **once per token**. From then on the token renders the scribe's saying in place of the theme.
   - Nothing is prepacked.
   - The contract derives the nine SMIL `values` lists and the `dur` from the text when the token is rendered (`cw_svg::marain::derived`).

## Roles
- **Collection admin** (the creator, owner of the cw721 creator role): runs `UpdateScribes { add, remove }`, with at most 50 changes per message. Each call emits a `wasm-scribes` event.
- **Scribe:** any address on the allowlist. A scribe may scribe any token that has no text yet. The token owner does not have to consent (see open decisions).

## Inputs a scribe sees for a token
- `token_id`.
- The minted theme: the current `scribe` param from `nft_info`, or `svg_token_uri`.
- The token's seed facts: collection `seed`, mint height, and `token_id`. Together these determine the theme and the colours.
- The `Scribed { token_id }` query. If it returns a record, the token is already done and is final.

## Steps
1. List tokens that have no `Scribed` record.
2. For each token, read the theme. Write an original saying or short poem that answers the theme.
   - A local open model may suggest drafts. Run it with `OLLAMA_HOST=127.0.0.1:<port> ollama serve` and stop it afterwards. The scribe chooses and edits the final text.
3. Check the text locally: `svgc marain-scribe OUT <seed_hex> <token_n> "<text>"`.
   - It renders the token after the scribe call, using the same checks as the chain.
   - Read the caption and watch the animation in the SVG.
4. Send `{"update_extension":{"msg":{"scribe":{"token_id":"<id>","text":"<text>"}}}}`.
   - The `wasm-scribe` event records `token_id`, `scribe`, `theme` and `text`.
   - **This is an on-chain tx under the scribe's key, so a human sends it. Never from this skill.**
5. Log `{token_id, theme, text, scribe, tx}` in the scribe's batch file for provenance.

## Guards (enforced on chain)
- **Charset:** `a-z`, `0-7` (octal Marain digits), space, `.`, `,`, `'`, `-`.
  - No upper case, no `8` or `9`.
  - No leading, trailing or double spaces.
  - 1–64 bytes and 1–40 glyphs.
  - Anything else gets `Scribe text rejected`.
- **Write-once:** a second call gets `already has a scribed text`, whoever sends it.
- **Allowlist:** a non-scribe gets `Sender is not an allowlisted scribe`. Removing a scribe takes effect at once.
- **Instantiate checks** on the template:
  - every theme passes the same charset check;
  - all of `scribe.c0`..`scribe.c8` and `scribe.dur` are present;
  - links pass the link allowlist;
  - static paint only.

## Guards (curation, human)
- Sayings must be original. No quotes or close paraphrases of Iain M. Banks or any other author. No "Culture" branding.
- No real people, brands, slurs or sexual content, and no medical or drug claims.
- Call it "Marain-style" (community glyph table). Do not call it "official Marain".
- No rotation anywhere: a rotated or mirrored glyph is a different letter.

## Randomness limits (theme pick)
- The seed is deterministic and public. A minter can predict the theme for the next block and choose when to send the mint; a block proposer can choose the block height.
- `crates/randomness` is not used. It draws OS entropy (`getrandom`), which is non-deterministic and cannot run inside consensus wasm.
- **The theme is cosmetic and the scribe replaces it, so this grinding risk is accepted.** If themes ever gain rarity or value, switch to a nois-style beacon: an async drand callback, which is unbiasable but needs a proxy contract and fees.
