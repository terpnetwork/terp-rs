# syntax=docker/dockerfile:1
#
# hash-market-server — VE + Blossom BUD + Merkle trees + Nostr/BUD router
#
# Build (from crates/terp-rs workspace root):
#   docker build -t hash-market:local -f tools/hash-market/Dockerfile .
#   docker tag hash-market:local ghcr.io/terpnetwork/hash-market:latest
#
# Run (config mounted — local/dev):
#   docker run --rm -p 9090:9090 \
#     -v /path/to/config.toml:/etc/hash-market/config.toml:ro \
#     -v /path/to/data:/var/lib/hash-market \
#     hash-market:local
#
# Run (Akash/o-line — secrets via SFTP, never env):
#   SSH_PUBKEY set → entrypoint starts sshd, waits for config.toml push
#   oline hashmerchant upload --config secrets/config.toml ...

# ─────────────────────────────────────────────────────────
# Stage 1: Build the Rust binary
# ─────────────────────────────────────────────────────────
FROM rust:1.84-alpine AS builder
RUN apk add --no-cache musl-dev pkgconfig openssl-dev openssl-libs-static

WORKDIR /build

COPY Cargo.toml Cargo.lock /build/
COPY tools/hash-market/Cargo.toml tools/hash-market/Cargo.lock /build/tools/hash-market/
COPY tools/hash-market/src /build/tools/hash-market/src/
COPY tools/onboard-authz /build/tools/onboard-authz/
COPY crates/nips/Cargo.toml /build/crates/nips/
COPY crates/nips/src /build/crates/nips/src/

# Default image: merkle/BUD host for NFT whitelist (no VE).
# For validator sidecar: --build-arg FEATURES=full-sidecar
ARG FEATURES=server,blossom,client,nostr
RUN cargo build --release \
    -p hash-market \
    --features "${FEATURES}" 2>&1

# ─────────────────────────────────────────────────────────
# Stage 2: Runtime (openssh for post-deploy secret delivery)
# ─────────────────────────────────────────────────────────
FROM alpine:3.21
RUN apk add --no-cache ca-certificates tzdata openssh-server su-exec \
    && ssh-keygen -A

RUN addgroup -S hashmarket && adduser -S hashmarket -G hashmarket

COPY --from=builder \
    /build/target/release/hash-market-server \
    /usr/local/bin/hash-market-server

COPY tools/hash-market/entrypoint.sh /usr/local/bin/entrypoint.sh
RUN chmod +x /usr/local/bin/entrypoint.sh \
    && chmod +x /usr/local/bin/hash-market-server

# Directories — config is empty until SFTP or volume mount
RUN mkdir -p /etc/hash-market /var/lib/hash-market \
    && chown -R hashmarket:hashmarket /var/lib/hash-market

ENV HM_CONFIG=/etc/hash-market/config.toml
ENV HM_DATA=/var/lib/hash-market

WORKDIR /var/lib/hash-market

EXPOSE 9090 22

# Root so sshd can start; entrypoint drops to hashmarket for the server.
ENTRYPOINT ["/usr/local/bin/entrypoint.sh"]
