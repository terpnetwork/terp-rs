# What is reusable in a Flock proof

Halo2 caches three objects. Flock caches two, and they are not the same
three.

| Halo2 | Flock, in this tree | Reuse |
|---|---|---|
| `Params` for a `k` | `flock_core::pcs::PcsParams` | Yes, across circuits that share the six fields below |
| Verifying key | `Registry::digest` | Per circuit. This is the circuit id: a hash of the registry, not a curve key and not a stored verifying key |
| Proving key | none | The prover uses the same registry and the same `PcsParams`. There is no proving key |

`PcsParams` is in `crates/flock/crates/flock-core/src/pcs/commit.rs`.
Every reusable field is one of these six. `LigeritoProfile` is an enum; it
has no fields of its own.

- `m` — bit-variable count of the committed polynomial. Packed length is
  `2^(m − 7)` F<sub>128</sub> words when `num_lanes` is `None`. It is not
  the log of the Merkle domain.
- `log_inv_rate` — Reed–Solomon inverse rate, `ρ = 2^{-log_inv_rate}`
  (`1` is rate 1/2, `2` is rate 1/4). It must equal
  `profile.log_inv_rate()`.
- `log_batch_size` — `2^log_batch_size` interleaved lanes. Each Merkle leaf
  holds one codeword position across those lanes (across `num_lanes` when
  that is set). The field is the log, not the lane count.
- `profile` — `LigeritoProfile`: `Fast` (default), `Fast100`, `Slim`,
  `Slim100`, `Secure`. The variant selects the opening schedule (queries,
  out-of-domain samples, grinding). Those counts are not fields on
  `PcsParams` or `LigeritoProfile`. `Fast`, `Fast100`, and `Secure` use
  `log_inv_rate = 1`. `Slim` and `Slim100` use `2`. `Secure` is
  unique-decoding and takes no out-of-domain samples. A profile is not a
  circuit.
- `num_lanes` — `Option<usize>`. `None` commits all `2^log_batch_size`
  lanes. `Some(t)` commits `t` lanes, `1 ≤ t ≤ 2^log_batch_size`.
- `merkle_hash` — `HashKind` (`Sha256` or `Blake3`) backing the Merkle
  commitment. `HashKind`'s default variant is `Blake3`.

The witness commitment is a Merkle root of Reed–Solomon codewords. There
is no generator vector `G_0 … G_{2^k-1}`.

## What is circuit-specific

`Registry` (`schedule.rs`) is the circuit. It holds the table types, the
uniform row capacity `2^nu`, and, on boolean tables, the sparse matrices
`A`, `B`, `C`. `Registry::digest` is BLAKE3 in this order:

1. domain `flock-registry-v1`;
2. format-version byte `1u8`;
3. `nu` as u32 little-endian;
4. type count as u32 little-endian;
5. each type in slot order: `k_log` (u32 LE), `useful_bits` (u64 LE),
   `const_pin` as `(present: u8, value: u64 LE)`, then `a_0`, `b_0`, `c_0`;
6. element types only, after that type's stub matrices: the label
   `flock-element-class-v0` and `ElementTableType::digest`;
7. a non-empty IO schema only, after the element payload when both exist:
   the label `flock-io-schema-v0`, the entry count, then each word's column
   and direction. A type with an empty schema absorbs nothing here.

That digest is the circuit id. Two frontends produce the same proof system
only when they produce the same digest and the same `PcsParams`. Sharing
`PcsParams` alone is not enough. There is no proving key to copy from one
circuit onto another.

The headstash claim uses registry capacity `nu = 8` and one `PcsParams`:
`m` from the union's committed-word count (`log2(committed words) + 7`),
`log_inv_rate = 1`, `log_batch_size = 6`, `LigeritoProfile::Fast`,
`num_lanes` from that same union at `log_batch_size = 6` (`None` when every
lane is full), and the default `merkle_hash` (`Blake3`). The table is the
Blake3 gate from `flock-prover`. Its registry digest is the circuit id.
The Fiat–Shamir domain is `terp-headstash-flock-v1`.

## Footer

`CircuitFooter` (`crates/cosmwasm/packages/zk/src/footer.rs`) is 80 bytes
(`zk_cosmwasm::COSMWASM_FOOTER_LENGTH`). The blob is
`[params | cs+vk | footer]`.

Fields: `prover_id`, `curve_id`, `k`, `i`, `param_len`, `cs_len`,
`vk_len`, `param_checksum`, `vk_checksum`.

- `param_len` / `param_checksum` — the reusable object. For Halo2 that is
  the IPA params. For Flock it is the bytes of one `PcsParams`. Many
  circuits share one param file when the six fields above match.
  `param_len = 0` is the empty-param convention (`prover_id` =
  `CircuitType::Groth16`, `param_checksum` = SHA-256 of the empty string).
  It is not a `PcsParams` value and must not be what makes a footer mean
  "Flock".
- `cs_len` — byte length of the constraint system. For Flock that is the
  encoded registry. Cache it by `Registry::digest` (that hash already binds
  the matrices, and the element payload and IO schema when present). Do not
  rebuild it per proof.
- `vk_len` / `vk_checksum` — byte length and checksum of the verifying-key
  body. That body is Halo2's verifying key. Flock has no verifying key and
  no proving key. The circuit id stays `Registry::digest`. Do not write
  that digest into the vk slot and call it a key.

`appstate_key` is the big-endian u32 `[prover_id, curve_id, k, 0]` (the
code passes `curve_id`, not a separate param id). Flock has its own
`prover_id`. `k` is the Halo2 row budget and is `0` when unused. The
analogous Flock size is `m` on `PcsParams`, and it belongs in the param
bytes, not in that byte, unless a later footer version says otherwise.

The host dispatch today (`zk-hosts` `flock.rs`) calls `verify_ligerito`
on one hard-coded R1CS and one hard-coded `PcsParams`. It does not load a
registry. A modular host loads `PcsParams` by `param_checksum` and the
registry by `Registry::digest`, then calls `verify_ligerito_union_circuit`.
The headstash claim is one such registry. It does not replace the Halo2
param cache.

## Acceptance

A change is done when all of these are true:

- One `PcsParams` value is encoded to the same bytes on two processes, and
  a second circuit with the same six fields loads those bytes unchanged.
- `Registry::digest` changes when a table matrix changes, and does not
  change when only the witness changes.
- A Flock footer round-trips through `CircuitFooter` and does not collide
  with a Halo2 footer of the same `k`.
- `cargo test -p headstash-claim --lib` passes.
- A local cw-orch or ict-rs run submits a proof receipt, the sidecar
  verifies it, and the only value written for consensus is the batch root.
  No proof bytes are in contract state. Nothing is broadcast.
