#!/bin/sh
set -e  # Exit on any error
echo "Starting Geth initialization for network: $NETWORK"

# Create data directory if it doesn't exist
mkdir -p "$DATA_DIR"

# JWT
secret_path="${GETH_AUTHRPC_JWTSECRET}"
mkdir -p "$(dirname "${secret_path}")"
jwt_secret="${RPC_JWT}"
echo "${jwt_secret}" > "${secret_path}"
chmod 600 "${secret_path}"

# snapshot
echo "Downloading snapshot from: $SNAPSHOT_ENDPOINT"
if ! wget -q --show-progress -O "$SNAPSHOT_FILE" "$SNAPSHOT_ENDPOINT"; then
    echo "Error: Failed to download snapshot"
    exit 1
fi
echo "Download complete. Extracting snapshot to: $DATA_DIR"
if ! tar -I zstd -xf "$SNAPSHOT_FILE" -C "$DATA_DIR"; then
    echo "Error: Failed to extract snapshot"
    exit 1
fi
rm -f "$SNAPSHOT_FILE"
echo "Snapshot extraction complete. Data directory ready."
 
# geth node command
CMD="geth --$NETWORK --datadir $DATA_DIR --http --http.addr $GETH_HTTP_ADDR --http.port $GETH_HTTP_PORT --authrpc.port $GETH_AUTHRPC_PORT --http.api eth,net,web3 --authrpc.jwtsecret $GETH_AUTHRPC_JWTSECRET --nodiscover=true"
 

echo "RPC endpoint enabled and exposed on $GETH_HTTP_ADDR:$GETH_HTTP_PORT"
echo "Starting Geth with command: $CMD"
exec $CMD