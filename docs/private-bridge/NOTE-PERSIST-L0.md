# Headstash note persistence — L0/L1/L2 test targets

**Core product status: CLOSED** (2026-07-20) — see
[reviews/CORE-PRODUCTS-CLOSE-2026-07-20.md](./reviews/CORE-PRODUCTS-CLOSE-2026-07-20.md).
**Ops notes plane: stable** (live `notes_base` + auth + snap recover + optional `sha256`) — see § Ops product below.

SSOT store: `HeadstashStore` at `notes/{hs_id}/{addr}.json` (`crates/terp-rs/tools/hash-market`).

Envelope (opaque to server): `{ciphertext, nonce, scheme, cleartext_layout?, cleartext_len?}` — requires non-empty string fields `ciphertext` / `nonce` / `scheme`. Private notes are **not** dual-indexed under public `/content`.

## hash-market (store + HTTP)

```bash
cd crates/terp-rs/tools/hash-market
cargo test --lib headstash_note --features server
cargo test --lib note_persist --features server
```

| Test filter / name | Locks |
|--------------------|--------|
| `set_note_get_note_round_trip` | set → get; layout 382; list + blobs |
| `multi_note_list_ordering_and_equal_pir_padding` | sorted keys; equal-length PIR pads |
| `set_note_overwrite_same_addr_replaces_envelope` | same addr replaces body |
| `set_note_rejects_empty_ciphertext` / `_missing_scheme` / `_non_object` | envelope validation |
| `validate_id_rejects_slash_spaces_dotdot` | charset + `..` |
| `get_note_blobs_xor_pir_select_index` | `xor_pir` over two stored notes |
| `note_persist_bridge_cm_addr_under_season_slug` | `cm.<hex>` path under season slug |
| `note_persist_http_put_get_with_auth` (L1) | PUT+auth → GET body |
| `note_persist_http_put_without_auth_401` (L1) | missing Bearer → 401 |
| `note_persist_http_get_missing_404` (L1) | missing note → 404 |

## zk-test-press harness (design usage, Docker-free)

```bash
cd crates/headstash/test-press
cargo test --lib note_persist --features 'interface,l0-seams'
# or without interface:
cargo test --lib harness::note_persist_l0 --features l0-seams
```

| Test | Locks |
|------|--------|
| `note_persist_bridge_l0_cm_addr_round_trip` | bridge happy → `cm.`+hex(cm_public) → mini-store round-trip |
| `note_persist_compose_seam_envelope_meta` | 382B SEAM + opaque envelope meta |
| `note_persist_validate_id_rejects_path_chars` | `/`, spaces, `..` |
| `note_persist_l0_seams_decode_then_envelope` | optional `l0-seams` decode + opaque persist |
| `note_persist_l0_seams_encrypt_store_decrypt_roundtrip` | real `encrypt_note_out` → MiniNoteStore → `decrypt_note_out` == 382B |

## Client writer (fixture crate)

```bash
cd crates/terp-rs && cargo test -p terp-seams --lib
```

| Surface | Role |
|---------|------|
| `encrypt_note_out` / `decrypt_note_out` | XChaCha20-Poly1305 over full SEAM 382B |
| `persist_plan_from_seam_note` | envelope + `notes/{hs_id}/{addr}` path |
| `put_note_envelope` (feature `http`) | blocking `PUT /notes/...` |
| `terp_seams::compose::persist_plan_after_bridge_mint` | default addr = `cm.`+hex(cm_public) |

## Product call site + L2 smoke (landed)

```bash
cd crates/headstash/test-press
cargo test --lib product_call_site --features 'interface,l0-seams'
cargo test --lib l2_e2e_bridge_mint_note_persist --features 'interface,l0-seams'
# live HTTP (optional): --features 'interface,l0-seams,note-http' + notes_base URL
```

| Surface | Role |
|---------|------|
| `NotesPersistConfig` | `hs_id`, `owner_key`, `notes_base` **or** `data_dir`, auth headers |
| `put_note_after_mint` | **Call site after BridgeMintNote/claim** — encrypt → put |
| `get_and_decrypt_note` | Owner recover (local or HTTP GET) |
| `PrivateBridgeSuite::e2e_l2_bridge_mint_note_persist` | Mock mint → put → get → decrypt |
| `terp_seams::dex::note::get_note_envelope` (feature `http`) | Live GET |

## Ops product (stable basis — 2026-07-20)

| Surface | How |
|---------|-----|
| Live `notes_base` + auth | `NotesPersistConfig::http` + `auth_headers`; server `[notes_auth]` |
| Bearer / snap auth | `terp_seams::dex::note::bearer_auth_headers` / `snap_secp_auth_headers` (feature `auth`); env `NOTES_BEARER_TOKEN` / `NOTES_SECP_SK` |
| CLI | `cargo run -p terp-seams --features cli --bin headstash-notes -- put|get|list|recover|pir-recover` |
| Snap recover | `recover` (GET+decrypt), `pir-recover` (list+PIR+decrypt); harness `recover_note` / `recover_note_via_pir` |
| Ciphertext `sha256` | `EncryptNoteOpts.include_ciphertext_sha256` / CLI `--with-sha256` / `with_ciphertext_sha256(true)`; server pins BUD blob when `[distribution]` on |

Server config (hash-market):

```toml
[notes_auth]
mode = "bearer_or_secp"   # noop | bearer | secp | bearer_or_secp
bearer_token = "…"
allowed_pubkeys = ["02…"]
```
