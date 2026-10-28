# Pre-UIP: dex-quote, posted liquidity over IBC v2

Draft for forum.penumbra.zone. Not posted.

Penumbra's spike `ibc-v2` branch can route one `AppHandlerV2` by destination port. This note asks for a port named `dex-quote` that answers a quote from public position state. It does not wrap ICS-20, does not parse the transfer memo, and does not run a batch swap inside `recv`.

## Why a port, not a transfer memo

The callbacks memo on an ICS-20 packet is an execution hook. A remote chain that wants a price back as data needs a request and an acknowledgement. Penumbra's shielded batch swap queues the fill and writes `BatchSwapOutputData` at the end of the block, so the recv acknowledgement cannot carry an executed price. The quote is the position a liquidity provider already posted: position id, pair, reserves, and the rational price `p/q`.

`LiquidityPositionsByPrice` is a query-service method. The handler reads that same component state directly. The price index is nonverifiable, so the acknowledgement the state machine writes is what the counterparty relies on.

## Payload

Port `dex-quote`. Version `dex-quote-1`. Encoding `application/json`.

```json
{
  "market_id": "PEN/USD",
  "height": 12345,
  "pair": { "asset_1": "penumbra", "asset_2": "usdc" },
  "position_id": ""
}
```

`market_id` and `height` are the sender's filing key. Penumbra does not interpret them. An empty `position_id` means the best opened position selling `asset_1` for `asset_2`. A non-empty id must be that opened position, in `plpid1…` bech32, or the handler returns an error. Asset ids are `passet1…` bech32, a known unit (`gm`, `gn`, `penumbra`), or another base denom. An ICS-20 trace such as `transfer/channel-0/uterp` is a base denom. The staking token's quote string is `penumbra`.

One packet carries one payload. Several quotes share it:

```json
{
  "quotes": [
    {
      "market_id": "PEN/USD",
      "height": 12345,
      "pair": { "asset_1": "penumbra", "asset_2": "usdc" },
      "position_id": ""
    },
    {
      "market_id": "PEN/ATOM",
      "height": 12345,
      "pair": { "asset_1": "penumbra", "asset_2": "atom" },
      "position_id": ""
    }
  ]
}
```

At most 16 quotes. An empty list, a larger list, a bad version, or JSON that is neither one quote nor this envelope is an error from the handler. A duplicate `(market_id, height)` inside the list is an error on the later row only.

## Acknowledgement

Success is one JSON object, the application acknowledgement bytes (not an ICS-20 `{"result":...}` envelope):

```json
{
  "position_id": "pos-1",
  "pair": { "asset_1": "penumbra", "asset_2": "usdc" },
  "reserves": { "r1": "1000", "r2": "2500" },
  "p": "5",
  "q": "2"
}
```

`p` and `q` are unsigned integers. The price is the rational `p/q`, with `p` the valuation of `asset_1` and `q` the valuation of `asset_2`. Reserves are that position's posted reserves in the same order. There is no note, no swap flow, and no clearing price.

A batch acknowledgement is one object with a `quotes` array. Each row echoes `market_id` and `height`. A hit has the same price fields as the single acknowledgement. A miss has `error` and no price:

```json
{
  "quotes": [
    {
      "market_id": "PEN/USD",
      "height": 12345,
      "position_id": "plpid1…",
      "pair": { "asset_1": "penumbra", "asset_2": "usdc" },
      "reserves": { "r1": "1000", "r2": "2500" },
      "p": "5",
      "q": "2"
    },
    { "market_id": "PEN/ATOM", "height": 12345, "error": "missing position" }
  ]
}
```

A missing position on a single quote is an error from the handler. The existing v2 receive path discards the sub-transaction and writes the universal error acknowledgement `sha256("UNIVERSAL_ERROR_ACKNOWLEDGEMENT")` (`4774d4a575993f963b1c06573736617a457abef8589178db8d10c94b4ab511ab`). The sender must not treat those 32 bytes as a price. A batch that parsed still returns the object above, including rows that missed, so one absent position does not drop the other quotes.

## What this does not change

The v1 transfer port check stays. The ICS-20 memo stays unread. There is no trait-object port router and no callbacks middleware. A later memo path, if one is written, uses the flat object `{"dest_callback":{"address":"...","gas_limit":"..."}}` with a native handler name, because Penumbra has no wasm VM. `gas_limit` would be a local operation cap. That memo path is not this port.

## Counterparty

The sender is a contract on a chain that speaks IBC v2. It sets `source_port` to its `wasm2` port, `destination_port` to `dex-quote`, and one payload. On the acknowledgement it divides `p` by `q`. On the universal error acknowledgement or a timeout it records nothing.
