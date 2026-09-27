# Agent brief — Zakura local for Private Bridge (D6)

**Track:** E2E / HARNESS (Zcash node local)  
**Owner pattern:** same as ROUND3-HARNESS-E2E + DEMO-CORRIDOR-E2E  
**Freeze:** D6 ACCEPTED — preauth ZEC dest + **local Zakura**

## Goal

Wire **Zakura** (local Zcash node) into our e2e story so Private Bridge can:

1. Derive / validate a **real Zcash address** (UA or transparent) for preauth  
2. Bind it to `owner_binding` in `DepositIntentV0`  
3. Optionally prove node RPC is reachable from **ict-rs** / docker lab  
4. Document how dao-dao-ui PrivateCorridor consumes that address  

**Not:** mainnet ZEC send for this sprint. **Not:** replace BTC path.

## Source of truth (repo already vendors Zakura)

| Surface | Path |
|---------|------|
| Zakura monorepo | `crates/zakura/` (Zebra fork, `zakurad`) |
| Docker | `crates/zakura/docker/docker-compose.yml`, `docker-compose.zakura-regtest-e2e.yml` |
| Official install | `cargo install --locked zakura` / `zakuracore/zakura` Docker Hub |
| Verify | minisign key `RWTZkHOmfhxdQf43RZJyOawUNvMSlbPH539O9Y2Sir/ZHTihqnSO1RZn` |
| Corridor SSOT | `DEMO-CASHAPP-ZEC-CORRIDOR.md`, `DESIGN-DECISIONS-CORRIDOR-ACCEPTED` D6 |
| Harness | `zk-test-press` cashapp corridor, `docs/plans/spectrum/e2e/` |

## Deliverables

1. **`docs/plans/spectrum/e2e/zakura/`** (or oline play sibling):
   - README: run local Zakura (prefer **regtest** or **testnet** docker for e2e; mainnet snapshot is ops)
   - Compose that can sit next to corridor-lab (hash-market + zakura RPC exposed)
2. **`zakura_local` harness helper** (Rust in test-press or spectrum e2e scripts):
   - Detect RPC (`z_getnewaccount` / zcashd-compat RPC as available)
   - Generate or fetch a receive address for demo dest
   - Hash to `owner_binding` using same domain as UI (`terp-dest-binding-v0|…` or document shared helper)
3. **Test** (Docker optional `#[ignore]` if no image):
   - Without Zakura: skip with clear message  
   - With Zakura up: get address → build intent dest → I5-style allows_swap still OK  
4. **ict-rs hook** (minimal): document ChainSpec or sidecar env for Zakura container if pattern exists; else docker-compose only with honest residual  
5. Point PrivateCorridor docs: “paste UA from local Zakura” + optional env `ZAKURA_RPC`

## Constraints

- Prefer **docker zakuracore/zakura** or monorepo `docker-compose` over compiling whole Zakura in CI  
- Lab banner remains if corridor is lab_simulated  
- Do not claim mainnet ZEC  
- Keep D1 SeamNoteOutV0 only  

## Done when

```bash
# document one of:
docker compose -f crates/zakura/docker/docker-compose.zakura-regtest-e2e.yml up -d
# or docker run zakuracore/zakura …

# harness:
cargo test -p zk-test-press --lib zakura --features 'interface,l0-seams'  # or e2e script green
```

## Return

Paths, how UI gets a ZEC dest, green commands, residual for lightwalletd/wallets.
