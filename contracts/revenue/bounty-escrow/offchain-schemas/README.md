# terp-bounty JSON schemas (DEV/TEST)

| id | file | what it is | on chain |
|---|---|---|---|
| `terp-bounty-details/v1` | `terp-bounty-details.v1.schema.json` | full bounty text, acceptance criteria, evidence required, reward quote ("proposed, pending vote") | `details_hash` (sha256 of the canonical bytes, required) + `details_cid` (optional) |
| `terp-bounty-submission/v1` | `terp-bounty-submission.v1.schema.json` | the hunter's work: evidence links, criterion-by-criterion self-check | `submit{submission_hash, cid}` (cid required, validated on chain) |

JSON Schema 2020-12, `additionalProperties: false`, and a `schema` const for versioning. A breaking change means a new
`/v2` file; consumers dispatch on `schema`.

**Canonical bytes:** UTF-8 JSON with object keys sorted and no whitespace (`lib.mjs canonicalJson`).
`hash = sha256(bytes)`. The CID we produce is `rawCidV1(bytes)` (`bafkrei…`: CIDv1, raw codec 0x55, sha2-256). That
is what `ipfs add --cid-version 1 --raw-leaves` returns for a file of one block (<256 KiB), so the CID's digest
*is* the hash and anyone can check it without a node. The contract also accepts other CIDs (`Qm…` v0, v1
dag-pb/dag-cbor/dag-json/json; sha2-256 or blake3). For those the backend checks only `sha256(fetched bytes) == submission_hash`.

**Checks JSON Schema cannot express** (done in `lib.mjs` and in the contract):
- `summary` ≤ 280 **bytes** of UTF-8.
- `bounty_ref`, `escrow.{contract,bounty_id,chain_id}` and `submitter` match the chain event.
- Acceptance evidence indexes are in range.
- The document is ≤ 256 KiB.

Build: `cd tools && npm i && npm run build && npm test`. `build.mjs` compiles the schemas to standalone ajv code
(`validators.gen.mjs`, so no `eval` at runtime) and bundles `lib.mjs` into `dist/terp-schemas.mjs` (copied to
`graph/app/v3/vendor/` by `graph/tools/bundle-escrow.sh`) and `dist/terp-schemas.cjs` (vendored by the backend). `examples/`
holds documents that pass validation.
