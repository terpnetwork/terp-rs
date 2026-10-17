# SKILL: Latent Benders: A Study — daily ASCII frame

> **Iteration 4 (supersedes the memory and disclaimer parts below).**
> - Renamed from "symbient" to **Latent Benders: A Study**. Persona: `personas/latent-benders.json`.
> - **Fresh daily draw, no memory.** `daily.py commit --day D` (before the day) writes a random 32-byte seed to
>   `seeds/D.seed` (0600) and records `sha256(seed)` + time in `state.json.commits`. `daily.py symbient-day` reveals
>   the seed, checks the hash, and draws 1–2 substances from the **unused** pool (Fisher-Yates keyed by
>   sha256(seed‖i)) plus one practice. Interactions are computed only among today's picks. `state.used_substances`
>   grows; when fewer than 2 remain the run stops with `substance pool exhausted`. The record stores `draw`
>   (commit, seed, times, pool before/after).
> - **Disclaimer only on the website.** Prompts, ASCII art, the SVG template, rich pages, narration and records carry
>   no "fictional / not dosing advice" text. The website pages show it. Amounts never appear (transcripts are
>   masked with DOSE_RE; narration may say a substance was taken).
> - **Off-chain frame (iteration 5): Kara & Gippy**, our own pair (primers: `personas/kara.md`, `personas/gippy.md`,
>   `personas/kara-gippy.md`). Session 1: trip conversation (Kara sits, reflects, asks open questions, never steers;
>   Gippy is fresh and shaped by the draw). Session 2 (fresh): they curate one ASCII painting (Kara = still frame,
>   muted fixed palette; Gippy = motion, hue from the revealed seed). Local model `qwen3.5:0.8b` (ollama
>   127.0.0.1:11535). `daily.py resession --out DAY` regenerates the frame for an existing day without touching the
>   on-chain day record. Site credit only: "inspired by wibandwob's symbient approach".
> - **Open mint per day:** price tier + `mint_end_time` (relative seconds) at instantiate; anyone can mint.


One collection = one persona (example: `personas/lumen-moss.json`). Each UTC day the harness builds one
packed-only cw721-svg collection (templates are immutable, so one collection per day). A token picks one of
that day's K frames (`${frame}`) and an ink/background palette.

> **Disclaimer (iteration 4: website only, not in the art):** Fictional generative art. Not dosing, safety or medical advice.
> Interaction labels are AI-assembled data from psychedex.org, shown with their review standing.
> "unknown" means unclassified and "unrated" means documented in prose without a grade. Neither ever means safe.

> **Selection is unfiltered by default (user decision, iteration 3).** The creator wants an uncensored record of
> documented effects, so the blocklist and the acute-toxicity filter no longer gate selection. Both stay in the code
> behind `--filters` (opt-in). What does not change: no doses are ever stored, prompted or shown. Severity, review
> standing and source are always shown. The disclaimer stays. `unknown` is never presented as safe.

## Inputs
- The persona JSON, the day (`YYYY-MM-DD`, UTC), and `state.json` (the hash chain, seed commitments and used substances; no memory since iteration 4).
- psychedex public pages:
  - `https://psychedex.org/llms-full.txt` (the slug list);
  - `https://psychedex.org/substances/<slug>.md`, cached on disk; the cache build waits at least 2 s between pages.
  - `https://psychedex.org/practices/<slug>` (HTML; there is no `.md` variant). Only the name, category, effect names,
    linked substance slugs, and the dose-free sentences of the "Substance Interactions" and "Contraindications" sections are kept.
  - Never `/api/`, which robots.txt disallows.
- Approved submissions in `queue/` (see `schemas/environment-submission.schema.json`).
- The local open model on 127.0.0.1.

## Data source (iteration 3)
- **Default `--source cache`:** the harness reads only `psychedex-cache-v2.json`.
  - The cache holds the first 30 days of picks and practices, plus the fields the harness needs for every substance and practice psychedex lists, with their source URLs.
  - It makes no network calls.
- **`--source live`:** use only after psychedex licensing is agreed. It switches to the public `.md` pages (still never `/api/`).
- **Rebuild the cache with `daily.py build-cache`.**
  - It honours robots.txt and waits at least 2 s between pages.
  - With `--filters` only: it applies the blocklist first, then the toxicity filter (acute toxicity must be Negligible or Low). Off by default.
  - It deletes the raw pages afterwards.
- **Blocklist (opt-in, `--filters`):**
  - opioids (any class containing "opioid");
  - PCP-type dissociatives (arylcyclohexylamines, plus pcp, rolicyclidine, pce and named analogues);
  - GHB, GBL and 1,4-butanediol;
  - NBOMes.

## Steps
1. `day_seed = sha256(persona | day | prev_record_sha256)`.
2. **Pick.**
   - Count: 1 + (u32(seed, 0) mod 2) substances.
   - Each pick: slug = sorted_slugs[u32(seed, i) mod N].
   - Every listed substance is eligible (filters off by default). With `--filters`, a pick is kept only if its acute toxicity is Negligible or Low and it is not blocklisted. At most 12 page fetches per day.
   - **Practice:** one practice per day. Cache: `days[i].practice`. Live: practices[u32(seed, 7) mod P].
3. **Memory.**
   - Each day, every remembered substance's weight is multiplied by 0.794 using integer math. This is a half-life of about 3 days. Drop any item below 100.
   - A new pick enters at weight 1000. A repeat pick adds 1000 (capped at 3000). This is the stacking rule.
4. **Interact.**
   - For each pair in (today's picks ∪ memory), find the most severe rule in either page whose label names the other substance or one of its classes.
   - Copy the rule's severity, standing, confidence and source URL.
   - No match gives `unknown`.
   - Severity becomes a visual motif and the frame's flicker:
     - lethal or dangerous: crack/static motif, strong flicker;
     - caution: interference motif, soft flicker;
     - unknown: fog.
4b. **Practice interactions.** For the day's practice and every active substance (picks ∪ memory), look for a sentence in the practice page's "Substance Interactions" or "Contraindications" prose that names the substance first, then one of its classes (for example "stimulants", or "MAOIs" via the alias table).
   - Each hit is recorded as `severity: "unrated"`, together with a `cue` (risk / attenuates / adjunct / mention), the sentence (`excerpt`) and the source URL.
   - psychedex practice pages carry no graded severity, so the harness never invents one.
   - The cue only steers the visual motif:
     - risk: grinding / and \ edges;
     - attenuates: faint dots;
     - adjunct: a calm = border;
     - mention: a thin thread.
5. **Setting.** Take the first approved submission for this day by `(paid_at, id)`. If there is none, use the persona's default setting.
6. **Generate** K=3 frames.
   - Frame k uses seed u32(day_seed, 100+k+1000·attempt), temperature 0.8, num_predict 1400, think=false.
   - Each output goes through `svgc sanitize`. If it is rejected, re-roll, up to 4 attempts. Record every rejected attempt.
7. **Rich render.** For each frame, `svgc rich-text` runs the same character filter and dose masking with no 48×24 clip (capped at 400×200).
   - The harness writes a script-free HTML page (CSP `default-src 'none'`) to the local content-addressed store `cas/<cid>`.
   - `<cid>` = `svgc cid` = CIDv1 raw sha2-256. This is identical to `ipfs add --cid-version 1 --only-hash` for pages up to 256 KiB, checked against kubo 0.43.1.
   - The CID becomes the frame's value in the `frame` options variable, so each token's params record its CID.
   - The SVG links to `https://${gateway}/ipfs/${frame}` and shows `ipfs://${frame}`.
   - Pinning or uploading the store is a separate, human-approved step.
8. **Build.**
   - `svgc symbient day-build.json OUT` sanitizes, writes the template, checks it like the chain does, packs it, and renders 3 example tokens.
   - Then `daily.py finalize` adds the hashes and `record_sha256`.
9. **Provenance.**
   - `record_sha256` becomes the collection `seed` (stored on chain). It covers the model digest, the seeds, and the prompt, raw-output and sanitized hashes.
   - The record is hash-chained to the previous day.
   - Publish the record JSON next to the collection.
10. Stop the model server. Instantiating the collection is a separate, human-approved step.

## ASCII prompt (v2, `daily.py symbient_prompt`)
```
You are <persona.name>, <persona.voice>
Draw ONE piece of ASCII art. Rules: at most 48 characters per line, at most 24 lines, only plain keyboard
characters, no words longer than 12 letters, no numbers, no doses, no instructions, no real people. Output only the art.
Today, drawn fresh: <Name (effect, effect)>; ...
Echoes from earlier days: <name fading (NN%)>, ...
Practice today: <practice name> (<category>; <effect names>). Let its rhythm shape the composition.
Practice and substance meet: <motif per practice_interaction cue>.        (only when documented)
Visual motif: <motif per substance-pair severity>.
Setting: <approved submission or persona default>.
Variation <k+1>.
```

## Sanitizer (deterministic, `src/ascii.rs`)
1. Strip ``` fences and normalize CR/LF line endings. Tabs become 4-column stops.
2. Only printable ASCII 0x20–0x7E is kept; anything else becomes `?`.
3. Drop leading blank lines. Keep the first 24 lines, clip each to 48 columns, and pad the grid to 48×24.
4. **Dose masking:** `<number>[ ]<unit>` (mg, mcg, ug, g, ml, tabs, hits, drops, …) is replaced by `~`.
5. Reject the frame if ink is below 15‰ or above 950‰.
6. XML-escape `& < >`. Rows are emitted as `<text>` elements under `white-space:pre`.

## Set/setting submissions (paid)

**Queue.** Submitter pays a fixed fee (proposed: 1 THIOL to the persona's payout address, memo = submission id). The fee buys **review**, not guaranteed use.

**Prefilter.** The deterministic rules M1–M3 in `daily.py prefilter`:
- M1: 8–280 characters;
- M2: ASCII only;
- M3: no URLs, handles, emails, phone numbers or dose patterns, and no buy/sell/vendor/dose words.

**Human moderation.** Reject anything that:
- names a real person or minor;
- is sexual or hateful, or harasses anyone;
- promotes or instructs drug use, sourcing, or combining substances;
- describes self-harm;
- contains an ad or spam;
- includes private data or a location that identifies someone.

Reasons are recorded as rule ids.

**Which day a submission applies to.**
- On approval it gets `target_day` = the earliest UTC day D such that:
  - approval happened before D−1 18:00 UTC, and
  - D has no other approved submission (1 slot per day, FIFO by `paid_at`).
- If D would be more than 14 days out, the submission becomes `refund_due`. Rejected submissions also become `refund_due`; refunds minus a review fee are an open decision.
- After the day is built, the submission is marked `applied`. Only its text sha256 goes into the day record. The text itself is published with the record.

**Payment verification.** Done by the operator (check the bank-send tx on chain). There is no contract for this yet; see open decisions.

## Link safety
- **On chain:** `cw_svg::links::validate_links` runs at instantiate. Every href must be one of:
  - `#id`;
  - `ipfs://<cid>[/path]`;
  - `https://{ipfs.io|dweb.link|w3s.link}/ipfs/<cid>[/path]`.
- Link variables must be options variables, and every combination is checked.
- `javascript:`, `vbscript:`, `data:text` and `data:application` are refused anywhere in the template or in option values.
- **On the page:** `lib/svg-sanitize.js` allows the same two external forms only on `<a>`, and adds `target=_blank rel="noopener noreferrer nofollow"`. External hrefs on `<image>` and other elements are still removed.

## Guards
- **No dose data:** doses are never parsed, stored or prompted, and model output is dose-masked.
- **Interactions** always carry severity, standing and source. `unknown` is never shown as safe.
- **Persona:** must be original. Do not impersonate Wib & Wob or any other existing symbient.
- Everything runs on 127.0.0.1 only. No chain txs from this skill.
