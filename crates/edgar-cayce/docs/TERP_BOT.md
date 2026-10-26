# Terp Network Bot Engine (edgar-cayce elevation)

Unified engine powering: RLM docs (`/terp ask`), validator monitoring
(`/validator`), governance feed (`/gov`), DAO notifier (`/dao`, foundation +
subDAOs only), bounties (`/bounty`), anti-spam (auto), daily dev digests
(`/terp digest-preview`).

Design refs (workflow only, Rust-native re-implementation):
- https://github.com/astrostake/discord-cosmos-validator-monitor — poll REST,
  diff jailed/status/missed, gov+upgrade alerts, slash-command management.
- https://github.com/DA0-DA0/discord-notifier-cf-worker — DAO webhook →
  Discord; here scoped by `DAO_ALLOWLIST` env to Terp Foundation only.
- https://github.com/DA0-DA0/discord-anti-spam-bot/blob/main/bot.py —
  keyword delete + timeout + notify channel; ported to `src/terp/spam.rs` +
  `SpamHandler` in `main.rs`.
- https://github.com/permissionlessweb/edgar-cayce — RLM core (`/edgar`
  kept for compat; `/terp ask` adds style layer).

## Style profile (returniflost)

1. Export your messages (Discord Settings → Privacy → Request Data, or a
   scraper with Message Content intent).
2. Normalize to `data/style/returniflost.json`: `["msg...", ...]` or
   `[{"content": "..."}]`.
3. Run `/terp profile-refresh` — shows n, avg_len, lowercase ratio, top tokens.
4. `/terp ask` injects the profile hint so answers + digests mirror cadence.

Never commit the corpus or tokens.

## Daily dev updates

- `/terp digest-preview lines:a | b | c` renders in-style digest.
- Scheduler: set `DIGEST_CHANNEL_ID` + `DIGEST_HOUR_UTC`; feed it commit
  subjects (e.g. `git log --since=24h --pretty=%s`). Background cron loop
  is the next step — currently on-demand + validator/gov poll tasks.

## Env additions

```
DAO_ALLOWLIST=terp1...,terp1...
SPAM_KEYWORDS=axvdex.org,astrovault airdrop
SPAM_NOTIFY_CHANNEL_ID=123
SPAM_TIMEOUT_DAYS=27
DIGEST_CHANNEL_ID=123
DIGEST_HOUR_UTC=16
```

## Storage (`src/docs/`)

sqlite + FTS5 (`data/docs/terp-docs.sqlite`), not JMT:

- **Docs**: blake3 content-addressed blobs (dedup + stable citation anchors),
  FTS5 inverted index over ~1200-char chunks (BM25 + match snippets) for the
  loop's search path. No more full-scan-per-call substring search.
- **Versions**: `(label, source)` → one current `doc_id`. Re-ingesting an
  updated doc supersedes instead of duplicating; `/edgar ask` and `/terp ask`
  only see current versions. Old blobs stay as history (`/edgar sources`
  lists everything).
- **Commitment log** (JMT's correct purpose, minimal form): every corpus
  mutation appends a hash-chained epoch; `/terp corpus-status` shows
  epoch + root. Publish roots later for answer attestation; a full JMT
  only earns its place with replication or proof verification.
- **Q/A two tiers**: every answer is logged (dataset/eval/export — never
  retrieved as a source). `/terp verify` (admin) promotes a logged answer;
  only verified records with an all-current doc set are re-servable, as
  labeled context, never primary evidence. `/terp verified` lists them.
  Re-ingest automatically retires stale verifications.

Migrating from the old cnidarium store: wipe `data/docs` (`just clean-data`)
and re-ingest — the old RocksDB files are inert once the sqlite file exists.

## Anti-spam

Delete + long timeout (27d default, Discord max 28d) + notify channel —
the DA0-DA0 pattern, so invite **with** ModerateMembers. Matching runs on
confusable-folded text: Cyrillic/Greek homoglyphs (`Оsmоsis … IОN Аirdrоp`)
are folded to Latin before keyword checks, and gift-emoji + airdrop/claim +
chain-or-token-name messages match even with zero URLs. Extra scam phrases
go in `SPAM_KEYWORDS` (comma-sep, appended to the built-ins).

## Validator monitor (automated, not slash commands)

A background task (needs `TERP_NODE_REST` + `VALIDATOR_ALERT_CHANNEL_ID`)
polls the full bonded set every `VALIDATOR_POLL_SECS` (default 60s) and
posts liveness messages like an internal Hermes watchdog:

- Missed blocks attributed exactly per validator (consensus pubkey →
  Tendermint address → bech32 `{prefix}valcons` → signing-info join).
- Offline episodes: first-seen timestamp, escalating warnings every
  `VALIDATOR_REMIND_SECS` (default 30m), slashing ETA from chain slashing
  params (`missed/allowed`), critical escalation near the threshold,
  tombstone/jail alerts, and back-online/recovering notices with downtime.
- `/validator list` is the on-demand read model (full set with 🟢🟡🔴💀
  flags). There is deliberately no per-validator slash query.

## Resource sync + live chain reads

Two speeds, one corpus:

**Static (synced).** `just sync-resources` materializes `bot-sync/` at the
repo root: parsed cargo dependency inventory per manifest, `cargo tree`
summaries, docs file listing, `state.json` static asset, and a
`chain-snapshot.json` (latest block, supply, inflation, pool, active
proposals — live only if `TERP_NODE_REST` is set at sync time). With the bot
running under `TERP_SYNC_ROOT=<terp-core>`, ingest via
`/edgar ingest-local path:bot-sync/... label:terp-deps|terp-docs|terp-chain`
(admin only; paths are confined to the sync root, binaries and lockfiles
skipped, per-file and per-doc size caps). Re-running the script + re-ingest
supersedes versions, which also retires stale verifications.

**Live (guard-railed).** The RLM loop gets `node_query(path)`, served
host-side through a nonce-scoped marker round-trip — the guest still has
zero network. All reads funnel through `src/terp/node.rs`: GET-only against the
operator-set `TERP_NODE_REST`, strict allowlist of read-only Cosmos LCD paths
(node_info, latest blocks, bank, staking, gov, auth, distribution, mint,
slashing, applied upgrade plan), pagination-only query params
(`limit` capped at 100, no `height`), 10s timeout, 32KB response cap with
pretty-printed JSON. Anything else returns a `refused:` string the loop can
see and adapt to — refusals are data, never exceptions. No tx submission,
no ABCI, no guest-influenced base URL, ever.

## Seed terp docs

```
/edgar ingest url:https://github.com/terpnetwork/terp-core label:terp doc_type:code
/edgar ingest url:https://terp.network label:terp
/terp ask question:how does ... topic:terp
```

## Execution isolation (`src/rlm/sandbox.rs`) — no opt-out

LLM-generated REPL code runs ONLY as a WASIX `python` guest inside
[`wasmer-sdk`](../../wasmer-sdk/rust). There is no in-process eval path;
no feature flag, no env toggle. Plain `cargo run` is the sandboxed binary.

Isolation properties:

- Guest runs with `NetworkPolicy::Disabled` — no sockets even if generated
  code imports them; FS access is confined to the sandbox.
- Corpus lives on a **read-only mount** (`/corpus/*.txt` + `manifest.json`,
  capped by `RLM_SANDBOX_SNAPSHOT_MAX`, default 8MB, seeded sizes recorded
  honestly, `snapshot_truncated` flag visible to the guest). History replays
  always see the same snapshot; the guest can only write its scratch
  workspace (`_run.py`).
- Tool surface (`list_files`, `read_file`, `grep`, `search_document`,
  `get_section`, `llm_query`, `node_query`) via a guest-side shim.
  `llm_query`/`node_query` are served host-side through **nonce-scoped**
  marker round-trips: each execution mints a fresh nonce, the host only
  honors markers carrying it, and caches are read live from `globals()` —
  so stale replays and marker strings inside document text cannot trigger
  host calls.
- Per-command timeout (`RLM_SANDBOX_TIMEOUT_SECS`, default 90) + 64KB
  output cap; REPL persistence emulated by re-running bounded cumulative
  history (`RLM_SANDBOX_HISTORY_MAX`, default 24KB).
- Python package ref is configurable (`WASMER_PYTHON_PACKAGE`, default
  `python/python` — pin e.g. `python/python@=3.13.18` for reproducibility).

Build requirements: rustc ≥1.95 (wasmer/cranelift floor; Docker uses
`rust:1.95-trixie`). No Python toolchain needed anywhere — generated code
never links host Python. The guest python package is fetched at session
spawn, so the host needs registry access at runtime.
