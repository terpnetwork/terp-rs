# ── Build stage ───────────────────────────────────────────────────────────────
# NOTE: wasmer/cranelift require rustc ≥1.95. No Python needed anywhere in
# this build: generated code runs in a WASIX guest, never in-process.
FROM rust:1.95-trixie AS builder

RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    clang \
    libclang-dev \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY src/ src/

RUN cargo build --release

# ── Runtime stage ─────────────────────────────────────────────────────────────
FROM debian:trixie-slim

RUN apt-get update && apt-get install -y \
    ca-certificates \
    libssl3 \
    curl \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

COPY --from=builder /app/target/release/discord-demo /usr/local/bin/edgar

RUN mkdir -p /app/data/docs

VOLUME ["/app/data"]

ENTRYPOINT ["edgar"]
