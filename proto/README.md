# `terp-proto` — protos and the Rust generator

This package is the **generator**, not a library. It has one binary:

```sh
cargo run -p terp-proto --bin proto-gen
```

It reads every `.proto` under this directory, writes Rust into
`crates/sdk/src/gen/`, and is feature-gated behind the workspace build — see
[docs/workspace-status.md](../docs/workspace-status.md) for the current state of
that build.

## Layout

| Path | Owner | Notes |
|------|-------|-------|
| `vendor/` | this repo | Third-party protos (cosmos, tendermint, ibc, amino, gogoproto, google, cosmwasm, capability, noble). Committed, hand-curated, **not** synced from anywhere. |
| `terp/`, `osmosis/` | **terp-core** | **Vendored copies.** Synced by the script below. Do not hand-edit. |
| `buf.lock` | **terp-core** | Copied by the same script so BSR deps track the chain. |

`proto-gen` walks this directory for chain protos and a hardcoded allowlist
under `vendor/` (see `src/main.rs`), with include paths `vendor/` then `.`.
That is why vendored protos live beside the chain protos rather than inside
`vendor/`: `import "terp/hashmerchant/v1/types.proto"` has to resolve.

## Syncing vendored protos from terp-core

`terp/` and `osmosis/` are copies. The chain is the source of truth — a proto
change lands in terp-core first, then syncs here.

```sh
scripts/gen/proto/vendor-terp-protos.sh          # sync + record manifest
scripts/gen/proto/vendor-terp-protos.sh --check  # drift check, no writes
```

The script copies only `.proto` files, and records the source commit plus a
per-file sha256 in [`vendor.sources.json`](vendor.sources.json). `--check` exits
nonzero on a byte difference **or** on a vendored file terp-core no longer
provides, so a stale module cannot survive a green run.

Overrides:

| Env | Default | Purpose |
|-----|---------|---------|
| `TERP_CORE` | `<repo>/../..` | terp-core checkout root. |
| `TSC_PROTO` | `$TERP_CORE/tests/tsh/upgrade/terp-core` | Source for `terp/clock`. |

### `terp/clock` is a pinned exception

terp-core dropped `x/clock` from its `proto/terp` tree, but
`crates/sdk/src/gen/terp.clock.v1.rs` is committed and `crates/sdk/src/lib.rs`
still declares the module. Regenerating without the protos would delete it and
break the SDK. So `terp/clock` is vendored from terp-core's pinned
`tests/tsh/upgrade/terp-core` tree, which is byte-identical to the last version
terp-core shipped. It is labelled `pinned:` in `vendor.sources.json`. When the
SDK drops the module, delete the fallback entry in the script at the same time.

## Rust codegen (this crate)

```sh
cargo run -p terp-proto --bin proto-gen
```

Requires the workspace to load — currently blocked; see
[docs/workspace-status.md](../docs/workspace-status.md). Output lands in
`crates/sdk/src/gen/`; review the diff before committing, since the post-
processing pass in `src/main.rs` strips derives by type name and those lists
go stale quietly.

## Go codegen (chain side, not used by this crate)

`buf` + the Go plugins produce chain-side Go, not the Rust this crate consumes.
Kept for parity with terp-core; `terp-rs` does not build or ship it.

Set the plugin versions first:

```sh
GOLANG_PROTOBUF_VERSION=1.36.6
GRPC_GATEWAY_VERSION=1.16.0

go install github.com/cosmos/cosmos-proto/cmd/protoc-gen-go-pulsar@latest
go install google.golang.org/protobuf/cmd/protoc-gen-go@v${GOLANG_PROTOBUF_VERSION}
go install github.com/grpc-ecosystem/grpc-gateway/protoc-gen-grpc-gateway@v${GRPC_GATEWAY_VERSION}
go install github.com/grpc-ecosystem/grpc-gateway/protoc-gen-swagger@v${GRPC_GATEWAY_VERSION}

git clone https://github.com/cosmos/gogoproto.git
cd gogoproto && go mod download && make install
```

Then, from the repo root:

```sh
cd proto && buf dep update && cd ..
buf generate

# move generated Go into place
cp -r ./github.com/terpnetwork/terp-core/x/* x/
rm -rf ./github.com
```

## Verifying the vendored protos without cargo

`buf` and `protoc` can check the vendored tree even when the workspace is
broken, which is how drift and import resolution are validated today:

```sh
scripts/gen/proto/vendor-terp-protos.sh --check

protoc -I proto -I proto/vendor --descriptor_set_out=/dev/null \
  $(find proto/terp proto/osmosis -name '*.proto')
```

`protoc` warns about unused imports in the upstream protos. Those warnings are
expected and are not drift — `vendor-terp-protos.sh --check` compares bytes, not
lint.
