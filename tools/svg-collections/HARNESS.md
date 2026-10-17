# Daily harness spec (shared)

- **Runtime:** a local open-weights model only.
  - ollama (`ollama serve` on 127.0.0.1, `/api/generate`, `stream:false`, `think:false`); or
  - llama.cpp `llama-server` on 127.0.0.1 (`/completion` with `seed`, `temperature`, `n_predict`). This needs an adapter (open item).
  - Record the model digest from `/api/tags`, or the GGUF sha256 for llama.cpp.
- **Determinism:**
  - Seeds come from the day seed.
  - Sampling is seeded, but output is only reproducible on the same model digest, runtime version and hardware. Provenance therefore stores *hashes of what was actually produced*, and does not rely on re-generating it.
- **Provenance on chain:**
  - The collection `seed` field = `record_sha256` (canonical JSON, sorted keys).
  - The record holds the model digest, per-frame seed and options, prompt sha256, raw sha256 and sanitized sha256, plus `packed_sha256` and `prev_record_sha256`.
  - Anyone can check the chain: hash the published record, compare it to the on-chain seed, and compare `svg_packed {}` to `packed_sha256`.
- **Scheduling:** none here (out of scope). A future cron runs: commit (before the day) → symbient-day (reveal + fresh draw + Kara & Gippy) → svgc symbient → finalize → human approval → instantiate.
- **Network:**
  - psychedex public pages only: substance `.md` pages and practice HTML pages. They are cached, fetched at most one every 2 s (robots.txt is honoured), with a UA that identifies the project. Never `/api/`.
  - The model runs on 127.0.0.1.
- **Selection filters:** off by default (user decision, iteration 3). `--filters` re-enables the blocklist and the acute-toxicity filter. Doses are never stored. Severity, standing and source are always shown.
- **Publishing:** `harness/publish.py` plans PUTs to the auto-pinned MinIO bucket (default `static`, prefix `svg-collections/`) on s3.terp.network, plus the expected https://ipfs.terp.network/ipfs/<cid> links. It is a dry run by default. Uploading needs `--execute --mc-alias <alias> --i-have-approval`.
- **Site data:** `harness/site_data.py` turns the cache and day records into `websites/svg-collections/data/symbient-days.json`.
- **Files:**
  - `harness/daily.py` (stdlib Python);
  - `svgc` (Rust: sanitize, build, pack, check);
  - `schemas/*.json` (day record v2 with `practice` and `practice_interactions`; `psychedex-pick-cache.schema.json`).


## Iteration 4
Renamed to **Latent Benders: A Study**. No memory; commit-reveal seed; no-repeat substances with exhaustion report; disclaimer removed from prompts/art/records (website only); Kara & Gippy transcript + painting in each CAS page (iteration 5; replaced Wib & Wob); open mint per day. See SKILL-symbient.md and ITERATION-4.md.


## Iteration 5
Original session pair Kara & Gippy replaces Wib & Wob (no borrowed persona text, no brain.wibandwob.com grounding). `daily.py resession` rebuilds the off-chain frame and CAS pages for an existing day; the day record (on-chain seed) is unchanged and `session.json` holds the new frame.
Painting pass: rows that only repeat the prompt's symbol list, verbatim repeat rows and model-added row labels ("WAVE 3:") are dropped. If every attempt is empty after that, a fallback prompt that names the marks in words (no glyph list) is tried at seed offsets 900+. Stray letters and digits become marks (`painting_texture_pass_lines` counts them).
