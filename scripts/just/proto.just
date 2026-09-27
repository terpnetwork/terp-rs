# scripts/just/proto.just
# Protobuf vendoring and Rust generation.
#
# proto/terp and proto/osmosis are vendored copies of terp-core/proto. Edit
# protos in terp-core and re-run; do not hand-edit them here.

proto_script := "scripts/gen/proto/vendor-terp-protos.sh"

# Sync vendored protos from terp-core and record proto/vendor.sources.json.
fetch-protos:
    bash {{proto_script}}

# Fail if the vendored protos drifted from terp-core. For CI.
check-protos:
    bash {{proto_script}} --check

# Regenerate crates/sdk/src/gen/ from the protos. Requires a loadable workspace.
proto-gen:
    cargo run -p terp-proto --bin proto-gen
