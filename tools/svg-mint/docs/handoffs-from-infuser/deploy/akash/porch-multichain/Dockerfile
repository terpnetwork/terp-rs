# One image, three ultra-light P2P daemons (BTC prune + ETH snap + Base snap).
# No public JSON-RPC. DREGG never queries this container.
FROM bitcoin/bitcoin:29.0@sha256:a6aa8a9e349b4108d13c558dbe43064057bd7b6474b858966884f9cb95b7ed78 AS btc
FROM ethereum/client-go:v1.15.11@sha256:798b7eb1bcef6d4be7576232beea63bf291450f48b025dc8fb5c6e37840e4364 AS geth
FROM us-docker.pkg.dev/oplabs-tools-artifacts/images/op-geth:v1.101511.0 AS opgeth

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates \
  && rm -rf /var/lib/apt/lists/*
COPY --from=btc /opt/bitcoin-29.0/bin/bitcoind /usr/local/bin/bitcoind
COPY --from=geth /usr/local/bin/geth /usr/local/bin/geth
COPY --from=opgeth /usr/local/bin/geth /usr/local/bin/op-geth
COPY keys/eth.nodekey keys/base.nodekey /etc/porch/
COPY entrypoint.sh /entrypoint.sh
RUN chmod 644 /etc/porch/*.nodekey && chmod 755 /entrypoint.sh \
  && mkdir -p /data
VOLUME ["/data"]
EXPOSE 8333 30303 30303/udp 30304 30304/udp
ENTRYPOINT ["/entrypoint.sh"]
