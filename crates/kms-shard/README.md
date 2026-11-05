# kms-shard (Plane S)

FROST-Ed25519 DKG + threshold signatures that verify as ordinary Ed25519. No WASI, no BLS, no sockets. The consensus scalar never leaves a `KeyPackage` share.

Scheme id: `frost-ed25519-v1`. Host (Go wazero) and DKG CLI come next; this crate is the scheme trait and vectors.
