FROM ethereum/client-go:latest

ARG NETWORK
ARG DATA_DIR
ARG RPC_ENABLE
ARG SNAPSHOT_ENDPOINT
ARG SNAPSHOT_FILE

ARG RPC_JWT
ARG GETH_AUTHRPC_ADDR
ARG GETH_AUTHRPC_PORT
ARG GETH_AUTHRPC_JWTSECRET
ARG GETH_HTTP
ARG GETH_HTTP_ADDR
ARG GETH_HTTP_PORT
ARG GETH_WS_ADDR
ARG GETH_WS_API


# -----------------------------------------------------------------
# Install tools required by the JWT helper (openssl, jq)
# -----------------------------------------------------------------
RUN apk add --no-cache openssl jq

# ---------- Runtime environment variables ----------
ENV NETWORK=${NETWORK}
ENV DATA_DIR=${DATA_DIR}
ENV RPC_ENABLE=${RPC_ENABLE}
ENV SNAPSHOT_ENDPOINT=${SNAPSHOT_ENDPOINT}
ENV SNAPSHOT_FILE=${SNAPSHOT_FILE}

# Geth‑specific env vars – the same names Docker/compose will set
ENV GETH_AUTHRPC_ADDR=${GETH_AUTHRPC_ADDR}
ENV GETH_AUTHRPC_PORT=${GETH_AUTHRPC_PORT}
ENV RPC_JWT=${RPC_JWT}
ENV GETH_AUTHRPC_JWTSECRET=${GETH_AUTHRPC_JWTSECRET}
ENV GETH_HTTP=${GETH_HTTP}
ENV GETH_HTTP_ADDR=${GETH_HTTP_ADDR}
ENV GETH_HTTP_PORT=${GETH_HTTP_PORT}
ENV GETH_WS_ADDR=${GETH_WS_ADDR}
ENV GETH_WS_API=${GETH_WS_API}

# Copy your custom initialization script
COPY entrypoint.sh /entrypoint.sh

# Make the script executable
RUN chmod +x /entrypoint.sh

# Set the custom script as entrypoint
ENTRYPOINT ["/entrypoint.sh"]