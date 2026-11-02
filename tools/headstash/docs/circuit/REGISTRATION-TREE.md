# Registration tree

Production claims use the K=11 membership circuit already measured on
`wip/headstash-halo2-noecdsa`. The circuit source stays as it is. Depth,
Poseidon tags, and the verifying-key id below are the lock. The K=19
ECDSA circuit remains a design variant at `vk_id` 1.

This file is the registration, anchor, and claim statement. `072f694`
locks the vk hash and the operator routes. The contract implements the
tree. Loopback code id 10 serves the page queries. Its wasm sha256 is
`1617a62c4e5977e3f649e6fd13fc89a0ae27ab408d2f5228cdbe3a2ce2003cda`.
One `RegisterHeadstash` on that code used 403982 gas. Code id 9, the
earlier wasm, used 398371 gas. Both numbers are transaction `gas_used`,
including the ante.

## Locked parameters

| Item | Value |
| --- | --- |
| Tree depth | 16. One campaign holds at most 2^16 leaves. |
| Poseidon | Pallas, `P128Pow5T3`, width 3, rate 2. |
| Tag encoding | `personalization_to_fp`: ASCII bytes at the front of a 32-byte array, the rest zero, then `pallas::Base::from_repr`. |
| `key_com` | Poseidon(`terp-hs-keycom-v1`, secret). `ConstantLength<2>`. |
| nullifier | Poseidon(`terp-hs-nf-v1`, secret, ctx). `ConstantLength<3>`. |
| leaf | Poseidon(`terp-hs-claim-leaf-v1`, key_com, amount). `ConstantLength<3>`. Amount is the u64 as a field element. |
| Merkle node | Poseidon(`terp-hs-distro-crh-v1`, layer, left, right). `ConstantLength<4>`. `layer` is the level as a field element, 0 at the leaves. |
| Field bytes | 32-byte little-endian `to_repr`. Hex in every signed string is 64 lowercase characters of those bytes. |
| `vk_id` | 2 |
| `vk_hash` | SHA-256 of params, then the post-synthesis constraint system, then the verifying key, then the 80-byte footer. K=11. Footer public-input count 6. Blob length 135023. Digest `94d4326108e28451829b68d64df58cf975fafdb874a5a554f3cbe8931eefed11`. |
| Proof | 2752 bytes at this depth. A depth of 20 or 24 is a new verifying key. |
| Public inputs | ctx, anchor root, nullifier, destination low, destination high, amount. |

The Merkle CRH tag is `distro_crh_tag()`, which is
`personalization_to_fp("terp-hs-distro-crh-v1")`. Leaf index bit 0 is
level 0. A clear bit means the running node is the left child. The empty
sibling is the zero field. A path shorter or longer than 16 is invalid
for a claim under this key.

The circuit hashes were measured at `7fb0f9d`. The whole-blob digest was
measured at `7077629` and written in `MEMBERSHIP-VK.md`. The campaign
stores that digest. The contract does not hard-code the bytes.

## Registration statement

The holder keeps `secret`. The holder computes `key_com` locally. The
chain never receives `secret`.

The eligible key signs Ethereum `personal_sign` over this ASCII string:

```text
headstash-register-v1:{ctx_hex}:{key_com_hex}:{amount}
```

`ctx_hex` is the campaign ctx. `key_com_hex` is the commitment.
`amount` is the eligibility amount as a decimal u64, with no sign and
no leading zeros. The campaign ctx is 32 canonical Pallas bytes. The
same bytes are the signature field and the claim public input.

The digest the contract recovers is keccak256 of the EIP-191 envelope:
the prefix `\x19Ethereum Signed Message:\n`, the decimal byte length of
the ASCII string, and the ASCII string. `Api::secp256k1_recover_pubkey`
takes that 32-byte digest, the 64-byte `r || s`, and a recovery
parameter of 0 or 1. The signature on the message is 65 bytes, `r || s || v`.
`v` of 0, 1, 27, or 28 is accepted. 27 and 28 become 0 and 1. Any other
`v` is a bad signature. The API returns the 65-byte uncompressed key,
`0x04 || X || Y`. The eligible address is the last 20 bytes of
keccak256(`X || Y`).

Anyone may relay `RegisterHeadstash`. The Cosmos sender is the relayer.
The eligible address is only the recovered address.

```text
RegisterHeadstash
  key_com:              32 bytes
  amount:               u64
  signature:            65 bytes
  eligibility_index:    u32
  eligibility_path:     one 32-byte sibling per eligibility level
```

The contract rebuilds the ASCII string from the stored campaign ctx and
these fields. A caller-supplied ctx is not a field.

Checks run in this order, and a failure leaves the frontier, the leaf
count, and both spent-sets unchanged:

1. The campaign has registration parameters and the tree has fewer than
   2^16 leaves. A full tree returns `tree full`.
2. `amount` is not zero. `key_com` is a canonical Pallas element.
3. The signature recovers. A failure returns `signature`.
4. The eligibility path for the recovered address, the amount, and the
   index opens the stored eligibility root. A failure returns
   `eligibility`.
5. That address has not registered. A repeat returns `already registered`.
6. That `key_com` is not already stored. A repeat returns `key commitment`.

The append happens after those checks. The assigned index is the leaf
count before the increment. The event attributes are `index` and `leaf`
(lowercase hex of the leaf `to_repr`). The event does not carry the
address, `key_com`, the amount, or `secret`. The transaction body still
carries the address's signature, `key_com`, and the amount, so an
observer who saved the transaction can join those three to the index.

## Eligibility tree

This tree is not the registration tree and it is not Poseidon. The
circuit never sees it. Its root and depth are fixed in the campaign
config. The relayer supplies the sibling path.

```text
leaf = SHA-256("terp-hs-elig-leaf-v1" || address_20 || amount_u64_le)
node = SHA-256("terp-hs-elig-node-v1" || left_32 || right_32)
```

`eligibility_depth` is an integer from 1 to 16. The path length is that
depth. The index is less than 2^depth. Bit 0 is the leaf level. A clear
bit places the running node on the left. The empty sibling is 32 zero
bytes, which is padding, not the hash of a zero address. The creator
builds the root with the same rule. The contract stores the root. It
does not store the address list.

## Append-only frontier

The contract stores 16 frontier field elements, the leaf count, and the
current root. It does not store every internal node. The empty tree has
count 0, a zero root, and 16 zero frontier elements.

`H(level, left, right)` is the Merkle node above. For a new leaf at
`index = count`:

```text
current = leaf
cursor = index
for level in 0..16:
    if cursor is even:
        frontier[level] = current
        current = H(level, current, 0)
    else:
        current = H(level, frontier[level], current)
    cursor = cursor >> 1
count = index + 1
root = current
```

A claim path is the sibling at each level. A missing sibling in that
path is the zero field. `auth_root` in the circuit uses the same bit
rule: a clear bit hashes `(node, sibling)`, a set bit hashes
`(sibling, node)`. A sidecar that applies the same events in chain
order produces the same root. The contract root is the source of truth.

## Accepted anchors

Campaign registration parameters:

| Field | Rule |
| --- | --- |
| `eligibility_root` | 32 bytes |
| `eligibility_depth` | 1 through 16 |
| `min_anonymity_set` | 1 through 2^16 |
| `anchor_start_height` | block height of the first window |
| `anchor_interval` | at least 1 |

`next_anchor_height` starts at `anchor_start_height`. `AcceptAnchor` is
permissionless.

It stores the current root when the block height is at least
`next_anchor_height`, the leaf count is at least `min_anonymity_set`,
and the current root is not the latest accepted root. It then sets
`next_anchor_height` to the current block height plus `anchor_interval`.
One call records one root. Missed windows do not create extra copies.

Any other call stores nothing and does not move `next_anchor_height`.
A call before the window returns `anchor early`. A short set returns
`anonymity set`. A repeated root returns `anchor unchanged`.

The accepted list is append-only. Older roots stay valid claim anchors.
The claim page defaults to the latest accepted root.

## Data flow

```text
holder                         relayer                    cw-headstash
  | secret stays local             |                            |
  | key_com = Poseidon(tag, secret)|                            |
  | personal_sign(register string) |                            |
  |------------------------------->| RegisterHeadstash      |
  |                                | path, index, amount    |
  |                                | key_com, signature     |
  |                                |----------------------->|
  |                                |                        | recover address
  |                                |                        | eligibility path
  |                                |                        | one address, one key_com
  |                                |                        | append leaf, store frontier
  |                                |                        | event: index, leaf
  |                                |<-----------------------|
```

```text
anyone                         cw-headstash
  | AcceptAnchor ------------->| height >= next window
  |                            | leaf_count >= min_anonymity_set
  |                            | root differs from the latest anchor
  |                            | append root, advance the window
```

```text
holder                     sidecar                         chain
  | secret, path, anchor       |                                |
  | prove in the Worker        |                                |
  | POST proof, message,       | rebuild frontier from events   |
  | denom, vk_id 2             | query root and accepted roots  |
  |--------------------------->| refuse a root that is absent  |
  |                            | refuse a rebuild that differs  |
  |                            | seal the batch under vk_id 2  |
  |                            |------------------------------->|
  |                            |                                | sudo pays the batch
  |                            |                                | row anchor is in the history
  |                            |                                | nullifier unused for this campaign
```

## Sequence

```text
participant H as Holder
participant R as Relayer
participant C as cw-headstash
participant S as Sidecar
H->>H: secret, key_com, personal_sign
H->>R: registration body
R->>C: RegisterHeadstash and eligibility path
C->>C: recover, open the eligibility root, append
C-->>S: event index and leaf
C->>C: AcceptAnchor once the window and the minimum set are met
S->>S: rebuild the frontier from the events
S->>C: query the root and the accepted history
S->>S: seal only when the rebuild matches and the anchor is stored
H->>S: membership proof under that anchor
S->>C: staged batch, then the accepted-root sudo
C->>C: anchor in history, nullifier fresh, vk hash matches, pay
```

## Claim and payout

The POST body stays `proof_hex`, `message`, `denom_hex`, and `vk_id` 2.
Halo2 claims carry no `signature_hex` and no leaf index. The message is
the circuit's canonical body:

```text
headstash-claim-v1:{ctx_hex}:{root_hex}:{nullifier_hex}:{destination}:{amount}
```

`destination` is the bech32 account, at most 62 bytes. It is packed into
the two public inputs by `pack_destination`: the first 31 bytes fill the
low field from the front of a 32-byte array, and the remaining bytes fill
the high field the same way. Amount in the message is the decimal u64.
The root in the message is the membership anchor, as `to_repr` hex.

The epoch accepted-root sudo stays the payout step. `attested_root` is
still the SHA-256 batch root the module already stores. It is not the
Poseidon root. The contract still does not run the Halo2 verifier. The
sidecar does.

A campaign without registration parameters keeps today's single
`EPOCH_PAID` flag and today's batch preimage. A registration campaign
adds a 32-byte `anchor` on each payout row and appends that anchor to
the row preimage after the amount. Settle and `ApplyAcceptedBatch`
reject a registration row whose anchor is missing or is absent from the
accepted history. The anchor may be an older accepted root. The latest
root is not required.

The nullifier key stays `{campaign root_id}:{nullifier hex}`. The anchor
is not part of the key. A second pay of the same nullifier returns
`nullifier already exists` and pays nothing, including when the new row
uses a different accepted root. The vk hash check stays in front of that
nullifier check. A registration campaign records each paid attested batch
root, so a later batch can settle. Paying the same attested batch root
again returns `epoch batch already paid`.

`SetEpochCampaign` for this path stores `vk_id` 2 and the whole-blob
`vk_hash` above. Both stay immutable after the first successful write,
including before any registration. A zero halo2 hash is refused. The
rules in `072f694` apply.

## Who may do what

Anyone may relay a registration. Anyone may call `AcceptAnchor` once the
window is open and the minimum set is met. A short set never becomes an
anchor. Claims against a root that is not in the history pay nothing.

Sidecar ingress stays public. Campaign install and seal stay
operator-signed, as specified for `/ve/headstash/campaign` and
`/ve/headstash/seal`. Any sidecar that has the events may be the ingress.
Sidecars do not register verifying keys and do not accept a root the
contract has not stored.

The holder generates `secret` in the browser, keeps it off the wire, and
stores it encrypted with a backup export. The wallet signs the
registration. The relayer submits it. The page waits for an accepted
anchor, defaults to the latest, proves in the Worker, and POSTs. The page
does not sign the payout. The Worker witness, the Merkle path, and
`secret` are not part of the POST body.

## Privacy

The proof does not reveal the leaf index. Against the proof alone, the
anonymity set is the leaves committed under that anchor.

`amount` is a public input, so the set that hides the holder is the
leaves under that anchor with the same amount. A unique amount selects
one leaf. `min_anonymity_set` and equal amounts are what make the set
real. Choosing the latest anchor is the page default.

Registration reveals the link from address to leaf to anyone who reads
the transaction. State records the address so a second registration can
be refused, and records `key_com` so a duplicate commitment can be
refused. A later claim does not carry the address. Computing the
nullifier requires `secret`. A unique amount joins the claim back to
that registration.

A sidecar that sees the proof sees the public inputs and the proof bytes.

## Page queries

The claim page polls these cw-headstash queries. CosmWasm JSON encodes
each `Binary` as standard base64. Roots are 32 bytes. On chain
`headstash-epoch-e2e` the live contract is code id 10,
`terp13we0myxwzlpx8l5ark8elw5gj5d59dl6cjkzmt80c5q5cv5rt54qw6thgf`.
The wasm sha256 is
`1617a62c4e5977e3f649e6fd13fc89a0ae27ab408d2f5228cdbe3a2ce2003cda`
(978264 bytes). One registration, `AcceptAnchor`, and `registration_path`
on that contract returned leaf count 1, one anchor with `leaf_count` 1,
path index 0, amount 15, and 16 siblings. Code id 9 does not serve
`registration_path`. The shapes below are code id 10.

`{"registration_tree":{}}` returns:

```json
{
  "count": 2,
  "root": "<base64>",
  "next_anchor_height": 12346,
  "min_anonymity_set": 1,
  "anchors": [
    {"root": "<base64>", "leaf_count": 1},
    {"root": "<base64>", "leaf_count": 2}
  ]
}
```

`count` is the number of leaves now. `min_anonymity_set` is the campaign
minimum. `anchors` is the append-only history. Each entry is the root
and the leaf count at the `AcceptAnchor` that stored it. A root is
claimable when it is one of those entries. `AcceptAnchor` stores an
entry only when `count >= min_anonymity_set` and the root is not already
the latest entry. The current `root` is the frontier tip. It is
claimable once an entry carries it. The page reads `count` against
`min_anonymity_set` to see whether a later `AcceptAnchor` can succeed.
That comparison does not make the current root claimable. The default
anchor is the last entry. Older entries stay claimable. An entry covers
leaf `index` when `index < leaf_count`.

`{"registration_path":{"key_com":"<base64>","anchor":"<base64>"}}`
returns the path of that key under that accepted root:

```json
{
  "index": 0,
  "leaf": "<64 lowercase hex>",
  "amount": 15,
  "siblings": ["<64 lowercase hex>", "... 16 values"]
}
```

`key_com` and `anchor` are the same 32 bytes as `RegisterHeadstash.key_com`
and `anchors[].root`. `index` is the append index. `leaf` is 64
lowercase hex of the 32-byte little-endian leaf, the same hex as the
`register_headstash` event attribute `leaf`. `amount` is the registered
decimal u64. `siblings` has length 16. Position 0 is the leaf level,
which is bit 0 of `index`. Each sibling is 64 lowercase hex of the
little-endian field (`to_repr`). A side with no leaf under that anchor's
`leaf_count` is 64 zero hex characters, the zero field, not a hash of an
empty subtree. A missing key returns `key commitment`. An unknown root,
or a root whose `leaf_count` does not cover `index`, returns `anchor`.

`denom_hex` in `POST /ve/headstash/claims` is 64 lowercase hex
characters, 32 bytes, with no `0x` prefix. Query `{"epoch_campaign":{}}`
and hex-encode the base64 `denom` field. Those bytes are
`blake3(raw_utf8)` with the last byte masked by `0x1F`
(`byte[31] &= 0x1F`), the same value as `derive_nd(raw)`. `raw` is the
token strategy's UTF-8 denom. On the loopback chain that string is
`uterp`. The membership circuit does not read this field. The sidecar
accepts the claim only when these 32 bytes equal the campaign `denom`.

## Gas

One registration does one `secp256k1_recover_pubkey`, one keccak256 for
the EIP-191 digest, one keccak256 for the address, `eligibility_depth`
SHA-256 node hashes, one leaf Poseidon, and 16 distro CRH Poseidons.
The Poseidon hashes dominate. This file does not guess the gas. A failed
check does not write the frontier.

On chain `headstash-epoch-e2e` at `127.0.0.1:36667`, code id 9
(`terp1vhndln95yd7rngslzvf6sax6axcshkxqpmpr886ntelh28p9ghuqkzx4sf`)
stored the contract before the path query and one successful
`RegisterHeadstash` used 398371 gas. `gas_wanted` was 200000000. The
figure is `gas_used` from `tx wasm execute`, including the ante. The
wasm stored for that measurement is sha256
`cadb3ea3aa5a0508ec8fbb912c7776a21ac25e7a99b4f01ec708fc9e487064e4`
(965535 bytes). Store, instantiate, and `SetEpochCampaign` on the same
code used 6296735, 274645, and 202779 gas. Those are not the
registration figure. The query after the transaction reported leaf
count 1. That code id does not serve `registration_path`.

Code id 10
(`terp13we0myxwzlpx8l5ark8elw5gj5d59dl6cjkzmt80c5q5cv5rt54qw6thgf`)
is the path-query wasm, sha256
`1617a62c4e5977e3f649e6fd13fc89a0ae27ab408d2f5228cdbe3a2ce2003cda`
(978264 bytes). One `RegisterHeadstash` used 403982 gas and the
following `AcceptAnchor` used 135673 gas. Both are `gas_used` from
`tx wasm execute`, including the ante. The path query is not a
transaction.

The contract hash has to equal `key_commitment`, `membership_leaf`, and
`poseidon_distro_crh` from the circuit crate on fixed inputs. The
registration test includes those vectors. The implementation adds that
native hash. It does not add a prover, and it does not edit the circuit.

## Failure modes

A bad signature, an ineligible address, a second registration of the
same address, and a repeated `key_com` are rejected. The frontier stays
as it was.

A reorg drops the registrations and anchors from the dropped blocks. The
frontier after the reorg is the contract state. A sidecar rebuilds from
the canonical event history and queries the contract root before it
seals. A rebuilt root that differs from the queried root is not sealed.
The sudo then cannot carry a root the contract has forgotten.

If a sidecar's rebuilt root differs from the contract at the same
height, the sidecar stops. It does not try to repair the contract. The
divergence is a missed event or a bug in the rebuild.

A full tree refuses the next registration. The frontier and the count
stay. There is no wrap and no second tree under the same campaign.

`AcceptAnchor` before the window, or below `min_anonymity_set`, stores
nothing. A claim whose anchor is missing from the history pays nothing.
A nullifier already stored pays nothing, including under a different
accepted root.

A vk hash other than the hash stored at campaign creation pays nothing.
That check stays in front of the nullifier write.

## Tests this statement requires

Contract unit tests, before any loopback run:

- a valid registration whose root matches an off-chain rebuild of the
  same append
- a bad signature, an ineligible address, a second registration, and a
  duplicate `key_com`, each leaving the frontier unchanged
- `AcceptAnchor` below `min_anonymity_set` storing nothing
- a claim under an older accepted root paying
- the same nullifier under a later accepted root returning
  `nullifier already exists` and paying nothing
- one successful registration used 398371 gas, recorded under Gas

Loopback RPC `127.0.0.1:36667` answers on chain `headstash-epoch-e2e`.
The wrapper rerun recorded `E2E_RESULT passed=17 failed=0` in `8aac6f9`
on code id 8. This registration work does not rerun that suite and does
not replace that wasm. The e2e script re-inits the chain home after a
failed status check, so it stays unused. The 4-validator Docker suite
is a later handoff. It is not started from this spec.
