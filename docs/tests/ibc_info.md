# IBC Asset Routing Engine

> **Agent entry:** `tests/agent/COMMANDS.md` · package `terp-scripts` · bin `ibc` · artifacts in repo-root `public/`.  
> Offline: `just scripts-ibc-offline` / `just scripts-ibc-validate`. Live generate needs `MAIN_MNEMONIC` (preflight first).

## Overview

The IBC Asset Routing Engine is a pre-mining system that computes all possible IBC denom hashes and trace paths for assets across interconnected chains. It builds a routing table from chain-registry asset lists and IBC connection data, producing lookup tables that power static websites, cross-chain dashboards, and IBC primitives.

we are specicially able to know: 
- the ibc denoms hash for `TERP` & `THIOL` for any path it is able to traverse via ibc
- the ibc denom hash for any set of known assets registered to a chain configured in our cw-orchestrator scripts
- the full ibc client path and graph information 

## Design Primitives

### 1. Chain Registry Spec Compliance

The engine follows two core chain-registry specifications:

**Asset List Schema** (`assetlist.schema.json`):
- Each asset has a `base` denom (the on-chain identifier)
- `traces` array describes the origin of the asset
- IBC traces contain `counterparty` (origin chain + denom) and `chain` (path on current chain)
- Native assets have empty traces or non-IBC trace types

**IBC Data Schema** (`ibc_data.schema.json`):
- Uses `chain_1` and `chain_2` keys (alphabetically ordered)
- Channels array uses `chain_1`/`chain_2` keys matching the root
- `tags.preferred` marks the canonical channel for a connection
- `tags.status` tracks channel lifecycle ("ACTIVE", "CLOSED")

### 2. Core Data Structures

```
┌─────────────────────────────────────────────────────────┐
│                    IBC Channel Graph                      │
│                                                           │
│  Adjacency list of chain connections                      │
│  Each edge: channel_id, counterparty_channel_id,          │
│            preferred flag, status                         │
│                                                           │
│  terp ──channel-6738──> osmosis ──channel-1──> terp       │
│  terp ──channel-115──> akash   ──channel-6──> terp        │
│  terp ──channel-13───> atomone ──channel-10──> terp       │
│  terp ──channel-393──> juno    ──channel-0──> terp        │
│  terp ──channel-235──> stargaze──channel-2──> terp        │
│  terp ──channel-100──> secret  ──channel-7──> terp        │
└─────────────────────────────────────────────────────────┘
                            │
                            ▼
┌─────────────────────────────────────────────────────────┐
│                 IBC Asset Routing Table                   │
│                                                           │
│  For each (dest_chain, ibc_denom):                        │
│    - symbol, origin_chain, origin_denom                   │
│    - trace_path (full IBC path)                           │
│    - route (ordered list of hops)                         │
│    - hop_count, preferred flag                            │
│                                                           │
│  terp:                                                    │
│    ibc/X...X -> AKT from akash (1 hop, preferred)        │
│    ibc/Y...Y -> ATONE from atomone (1 hop, preferred)    │
│    ibc/Z...Z -> UM from penumbra (2 hops, via osmosis)   │
│    ibc/W...W -> ETH from ethereum (3 hops, via osmo+hub) │
└─────────────────────────────────────────────────────────┘
                            │
                            ▼
┌─────────────────────────────────────────────────────────┐
│              Simplified Lookup Table                       │
│                                                           │
│  dest_chain -> ibc_denom -> {symbol, origin, path}       │
│  Optimized for O(1) static website lookups                │
└─────────────────────────────────────────────────────────┘
```

### 3. CW-Orch State Integration

The engine uses `cw-orchestrator`'s `DaemonState` for persistence:

```
~/.cw-orch/state.json
├── morocco-1/                    # Terp chain state
│   ├── assets/                   # Native + derived IBC assets
│   │   ├── ""                    # Native sdk.coin assets
│   │   └── ibc/                  # IBC assets with computed hashes
│   ├── ibc_data/                 # IBC connection data
│   │   ├── akash-terp/           # Connection entry (chain_1/chain_2 format)
│   │   ├── atomone-terp/
│   │   ├── osmosis-terp/
│   │   └── ...
│   ├── code_ids/                 # CosmWasm code IDs
│   └── default/                  # Default contract addresses
├── osmosis-1/                    # Osmosis chain state
│   ├── assets/                   # Source asset list for multi-hop derivation
│   └── ...
└── ...
```

### 4. IBC Denom Hash Derivation

The IBC denom on a destination chain is the SHA-256 hash of the full trace path:

```
trace_path = "transfer/channel-6738/transfer/channel-79703/upenumbra"
ibc_denom  = "ibc/" + SHA256(trace_path).toUpperCase()
           = "ibc/0FA9232B262B89E77D1335D54FB1E1F506A92A7E4B51524B400DC69C68D28372"
```

**Single-hop example** (AKT from Akash to Terp):
```
path = "transfer/channel-115/uakt"
hash = SHA256("transfer/channel-115/uakt")
```

**Multi-hop example** (UM from Penumbra to Terp via Osmosis):
```
path = "transfer/channel-6738/transfer/channel-79703/upenumbra"
         ^^^ terp->osmo       ^^^ osmo->penumbra
hash = SHA256("transfer/channel-6738/transfer/channel-79703/upenumbra")
```

**Triple-hop example** (ETH from Ethereum to Terp via Cosmos Hub and Osmosis):
```
path = "transfer/channel-6738/transfer/channel-0/transfer/08-wasm-1369/0xc02aaa...cc2"
         ^^^ terp->osmo     ^^^ osmo->hub    ^^^ hub->eth (Eureka)
```

## Function Reference

### Core Functions

#### `compute_ibc_denom_hash(trace_path: &str) -> String`

Computes the IBC denom hash from a trace path.

**Input**: Full trace path (e.g., `"transfer/channel-115/uakt"`)
**Output**: IBC denom with hash (e.g., `"ibc/1480B8FD20AD5FCAE81EA87584D269547DD4D436843C1D20F15E00EB64743EF4"`)

**Algorithm**:
1. SHA-256 hash the UTF-8 encoded trace path
2. Convert to uppercase hex
3. Prefix with `"ibc/"`

---

#### `derive_terp_ibc_denom(asset, terp_channels) -> Option<(ibc_hash, trace_path, counterparty_chain)>`

Derives the IBC denom for a foreign asset on the Terp chain.

**Logic**:
1. Extract the IBC trace from the asset's `traces` array
2. Get the counterparty chain name and base denom
3. **Direct route**: If Terp has a preferred channel to the counterparty chain:
   - Path: `transfer/{terp_channel}/{counterparty_base_denom}`
4. **Multi-hop route**: If no direct channel, route through the source chain:
   - Path: `transfer/{terp_channel_to_source}/{source_trace_path}`
5. Compute the IBC denom hash from the full path

**Example - AKT (direct)**:
```
Asset traces: counterparty=akash, base=uakt, source=osmosis
Terp channels: akash -> channel-115
Result: transfer/channel-115/uakt
```

**Example - UM (multi-hop)**:
```
Asset traces: counterparty=penumbra, base=upenumbra, source=osmosis
Terp channels: no direct to penumbra, osmosis -> channel-6738
Source trace: transfer/channel-79703/upenumbra
Result: transfer/channel-6738/transfer/channel-79703/upenumbra
```

---

#### `build_channel_to_chain_map(state: &DaemonState) -> HashMap<String, TerpChannelInfo>`

Builds a mapping from counterparty chain names to Terp's channel info.

**Source**: Reads from `state.ibc_data` entries
**Output**: HashMap keyed by counterparty chain name

**Example output**:
```
{
  "osmosis"   -> TerpChannelInfo { terp_channel_id: "channel-6738",  cp_channel_id: "channel-1"    },
  "akash"     -> TerpChannelInfo { terp_channel_id: "channel-115",   cp_channel_id: "channel-6"    },
  "atomone"   -> TerpChannelInfo { terp_channel_id: "channel-13",    cp_channel_id: "channel-10"   },
  "juno"      -> TerpChannelInfo { terp_channel_id: "channel-393",   cp_channel_id: "channel-0"    },
  "stargaze"  -> TerpChannelInfo { terp_channel_id: "channel-235",   cp_channel_id: "channel-2"    },
  "secret"    -> TerpChannelInfo { terp_channel_id: "channel-100",   cp_channel_id: "channel-7"    },
  "evmos"     -> TerpChannelInfo { terp_channel_id: "channel-101",   cp_channel_id: "channel-5"    },
  "gitopia"   -> TerpChannelInfo { terp_channel_id: "channel-1",     cp_channel_id: "channel-4"    },
  "omniflix"  -> TerpChannelInfo { terp_channel_id: "channel-33",    cp_channel_id: "channel-3"    },
}
```

**Channel selection priority**:
1. Preferred transfer channel (`tags.preferred: true`)
2. Any active transfer channel (fallback)

---

### IBC Channel Graph Functions

#### `IBCChannelGraph::build_from_state(state) -> IBCChannelGraph`

Constructs a bidirectional graph of IBC channel connections from state.

**Nodes**: Chain names
**Edges**: Channel connections with metadata

**Example graph**:
```
terp ─── channel-6738/channel-1 ─── osmosis
terp ─── channel-115/channel-6  ─── akash
terp ─── channel-13/channel-10   ─── atomone
terp ─── channel-393/channel-0   ─── juno
```

---

#### `IBCChannelGraph::find_routes(source, dest, max_hops) -> Vec<Vec<ChannelHop>>`

Finds all routes between two chains using BFS.

**Parameters**:
- `source`: Origin chain name
- `dest`: Destination chain name
- `max_hops`: Maximum number of hops (typically 3)

**Returns**: Routes sorted by (hop_count ascending, preferred channels first)

**Example**:
```
find_routes("akash", "terp", 3) ->
  [
    [ChannelHop { from: "akash", to: "terp", from_ch: "channel-6", to_ch: "channel-115" }]
  ]

find_routes("penumbra", "terp", 3) ->
  [
    [ChannelHop { from: "penumbra", to: "osmosis", ... },
     ChannelHop { from: "osmosis", to: "terp", ... }]
  ]
```

---

#### `IBCChannelGraph::compute_ibc_denom_for_route(origin_denom, route) -> (ibc_hash, trace_path)`

Computes the IBC denom for an asset traversing a specific route.

**Algorithm**:
1. Walk the route from destination back to origin
2. At each hop, prepend `transfer/{channel_id}/`
3. Append the origin denom
4. Hash the full path

---

### Routing Table Functions

#### `IBCAssetRoutingTable::premine(graph, chain_assets, max_hops) -> IBCAssetRoutingTable`

Pre-computes all possible IBC asset routes across all chains.

**Process**:
1. For each native asset on each chain:
   - Find all reachable chains within `max_hops`
   - Compute the IBC denom for each route
2. For each IBC asset (non-native):
   - Resolve its origin chain and denom
   - Find further routes to other chains
   - Compute multi-hop IBC denoms

**Output**: Complete routing table with all possible paths

---

#### `IBCAssetRoutingTable::lookup_ibc_denom(symbol, dest_chain) -> Option<&IBCAssetRoute>`

Forward lookup: Find the IBC denom for a symbol on a specific chain.

**Priority**: Preferred routes first, then any available route

**Example**:
```
lookup_ibc_denom("AKT", "terp") ->
  Some(IBCAssetRoute {
    symbol: "AKT",
    origin_chain: "akash",
    origin_denom: "uakt",
    dest_denom: "ibc/1480B8FD20AD5FCAE81EA87584D269547DD4D436843C1D20F15E00EB64743EF4",
    trace_path: "transfer/channel-115/uakt",
    hop_count: 1,
    preferred: true,
  })
```

---

#### `IBCAssetRoutingTable::reverse_lookup(ibc_denom, chain) -> Option<&IBCAssetRoute>`

Reverse lookup: Given an IBC denom on a chain, find the origin asset.

**Example**:
```
reverse_lookup("ibc/1480B8FD...", "terp") ->
  Some(IBCAssetRoute {
    symbol: "AKT",
    origin_chain: "akash",
    origin_denom: "uakt",
    ...
  })
```

---

#### `IBCAssetRoutingTable::to_simplified_lookup() -> serde_json::Value`

Exports a minimal lookup table optimized for static websites.

**Format**:
```json
{
  "terp": {
    "ibc/1480B8FD20AD5FCAE81EA87584D269547DD4D436843C1D20F15E00EB64743EF4": {
      "symbol": "AKT",
      "origin_chain": "akash",
      "origin_denom": "uakt",
      "trace_path": "transfer/channel-115/uakt",
      "hop_count": 1
    },
    "ibc/BC26A7A805ECD6822719472BCB7842A48EF09DF206182F8F259B2593EB5D23FB": {
      "symbol": "ATONE",
      "origin_chain": "atomone",
      "origin_denom": "uatone",
      "trace_path": "transfer/channel-13/uatone",
      "hop_count": 1
    }
  }
}
```

---

### Asset List Derivation Functions

#### `derive_ibc_asset_list(terp, ibc, state_terp, assetlist, ibc_data_map) -> Vec<serde_json::Value>`

Processes an asset list and derives IBC denoms for all assets.

**For each asset**:
1. Check if it has IBC traces
2. Call `derive_terp_ibc_denom` to compute the IBC denom on Terp
3. Build the full asset entry with computed hash
4. Non-IBC assets (bridges, factory tokens) use their base denom

---

#### `build_ibc_asset_entry(asset, ibc_hash, traces, trace_path, symbol, cp_chain, cp_denom, dest_channel, image_override) -> serde_json::Value`

Constructs a complete asset entry following the chain-registry assetlist schema.

**Output format**:
```json
{
  "base": "ibc/1480B8FD...",
  "symbol": "AKT",
  "name": "Akash",
  "display": "akt",
  "type_asset": "ics20",
  "denom_units": [
    { "denom": "ibc/1480B8FD...", "exponent": 0, "aliases": ["uakt"] },
    { "denom": "akt", "exponent": 6 }
  ],
  "traces": [{
    "type": "ibc",
    "counterparty": {
      "chain_name": "akash",
      "base_denom": "uakt",
      "channel_id": "channel-6"
    },
    "chain": {
      "channel_id": "channel-115",
      "path": "transfer/channel-115/uakt"
    }
  }],
  "images": [...],
  "logo_URIs": {...},
  "coingecko_id": "akash-network"
}
```

---

#### `load_assetlist_for_chain(chain_name, state) -> Vec<serde_json::Value>`

Loads assets from a chain's state and tags them with `_source_chain`.

**Purpose**: Tracks which chain the asset data came from for multi-hop routing.

---

### IBC Data Construction Functions

#### `derive_full_ibc_state(terp, ibc) -> Result<()>`

Main entry point. Orchestrates the full IBC state derivation:

1. Query all IBC connections from the chain
2. Build IBC data entries (chain_1/chain_2 format)
3. Write IBC data to state
4. Build channel map from updated state
5. Load source asset lists
6. Derive IBC denoms for all assets
7. Build the routing table
8. Export lookup tables

---

#### `build_channel_entries(channels, chain_1_name, chain_2_name) -> Vec<serde_json::Value>`

Constructs channel entries following the ibc_data schema.

**Schema compliance**:
- Uses `chain_1` and `chain_2` as keys (not chain names)
- Alphabetical ordering of chain names
- Includes `ordering`, `version`, `tags`

---

## Getting Started

### Prerequisites

```bash
# Install terpd
cargo install terpd

# Install cw-orchestrator
cargo install cw-orch

# Install hermes (for relaying)
cargo install ibc-relayer-cli
```

### Step 1: Download Chain Data

```bash
# Clone the chain-registry
git clone https://github.com/cosmos/chain-registry.git
cd chain-registry

# Asset lists are at:
# _chain_name_/assetlist.json
# IBC data is at:
# _chain_name_/ibc_data.json

# Key files for Terp:
ls terp/assetlist.json
ls terp/ibc_data.json
```

### Step 2: Reference Asset Lists by Location

Asset lists are sourced from:

| Chain | Asset List URL |
|-------|---------------|
| Terp | `https://raw.githubusercontent.com/cosmos/chain-registry/master/terp/assetlist.json` |
| Osmosis | `https://raw.githubusercontent.com/cosmos/chain-registry/master/osmosis/assetlist.json` |
| Akash | `https://raw.githubusercontent.com/cosmos/chain-registry/master/akash/assetlist.json` |
| Cosmos Hub | `https://raw.githubusercontent.com/cosmos/chain-registry/master/cosmoshub/assetlist.json` |

IBC data is sourced from:

| Connection | IBC Data URL |
|-----------|-------------|
| Terp-Osmosis | `https://raw.githubusercontent.com/cosmos/chain-registry/master/_IBC/osmosis-terp.json` |
| Terp-Akash | `https://raw.githubusercontent.com/cosmos/chain-registry/master/_IBC/akash-terp.json` |
| Terp-AtomOne | `https://raw.githubusercontent.com/cosmos/chain-registry/master/_IBC/atomone-terp.json` |

### Step 3: Run the IBC State Derivation

```bash
# Build the project
cargo build

# Run the IBC derivation script
cargo test --test ibc_info -- --nocapture

# Output files:
# - assetlist.json (Terp's full asset list with IBC denoms)
# - ibc_lookup_table.json (simplified lookup for static sites)
# - ibc_routing_table.json (full routing table with all paths)
```

### Step 4: Verify the Output

```bash
# Check the asset list
cat assetlist.json | jq '.assets[] | select(.base | startswith("ibc/")) | {symbol, base}'

# Check the lookup table
cat ibc_lookup_table.json | jq '.terp | keys'

# Check the routing table
cat ibc_routing_table.json | jq '.metadata'
```

## Static Website Integration Guide

### For Team Members: Using the Lookup Table

The `ibc_lookup_table.json` file is designed for static website consumption. Here's how to integrate it:

#### 1. Load the Lookup Table

```javascript
// Fetch the lookup table at build time or runtime
const lookupTable = await fetch('/ibc_lookup_table.json').then(r => r.json());

// Structure: lookupTable[destChain][ibcDenom] -> assetInfo
```

#### 2. Resolve an IBC Denom to Human-Readable Info

```javascript
function resolveIBCDenom(ibcDenom, chain = 'terp') {
  const info = lookupTable[chain]?.[ibcDenom];
  if (!info) return { symbol: 'Unknown', origin: 'Unknown' };
  
  return {
    symbol: info.symbol,           // e.g., "AKT"
    originChain: info.origin_chain, // e.g., "akash"
    originDenom: info.origin_denom, // e.g., "uakt"
    tracePath: info.trace_path,     // e.g., "transfer/channel-115/uakt"
    hopCount: info.hop_count,       // e.g., 1
  };
}

// Usage:
resolveIBCDenom("ibc/1480B8FD20AD5FCAE81EA87584D269547DD4D436843C1D20F15E00EB64743EF4");
// => { symbol: "AKT", originChain: "akash", originDenom: "uakt", ... }
```

#### 3. Find All Assets on a Chain

```javascript
function getAllAssetsOnChain(chain = 'terp') {
  const chainData = lookupTable[chain] || {};
  return Object.entries(chainData).map(([ibcDenom, info]) => ({
    ibcDenom,
    symbol: info.symbol,
    originChain: info.origin_chain,
    hopCount: info.hop_count,
  }));
}

// Usage:
getAllAssetsOnChain('terp');
// => [
//   { ibcDenom: "ibc/1480B8FD...", symbol: "AKT", originChain: "akash", hopCount: 1 },
//   { ibcDenom: "ibc/BC26A7A8...", symbol: "ATONE", originChain: "atomone", hopCount: 1 },
//   { ibcDenom: "ibc/0FA9232B...", symbol: "UM", originChain: "penumbra", hopCount: 2 },
//   ...
// ]
```

#### 4. Reverse Lookup (Symbol to IBC Denom)

```javascript
function findIBCDenom(symbol, destChain = 'terp') {
  const chainData = lookupTable[destChain] || {};
  for (const [ibcDenom, info] of Object.entries(chainData)) {
    if (info.symbol === symbol) return ibcDenom;
  }
  return null;
}

// Usage:
findIBCDenom('AKT', 'terp');
// => "ibc/1480B8FD20AD5FCAE81EA87584D269547DD4D436843C1D20F15E00EB64743EF4"
```

#### 5. Display Multi-Hop Route Information

```javascript
function getRouteInfo(ibcDenom, chain = 'terp') {
  const info = lookupTable[chain]?.[ibcDenom];
  if (!info) return null;
  
  const hops = info.trace_path.split('/').filter(p => p.startsWith('channel-'));
  
  return {
    symbol: info.symbol,
    originChain: info.origin_chain,
    isMultiHop: info.hop_count > 1,
    hopCount: info.hop_count,
    channels: hops,
    pathDisplay: info.trace_path
      .split('/')
      .reduce((acc, part, i) => {
        if (part === 'transfer') acc.push('→');
        else if (part.startsWith('channel-')) acc.push(part);
        else acc.push(part);
        return acc;
      }, [])
      .join(' '),
  };
}

// Usage:
getRouteInfo("ibc/0FA9232B262B89E77D1335D54FB1E1F506A92A7E4B51524B400DC69C68D28372");
// => {
//   symbol: "UM",
//   originChain: "penumbra",
//   isMultiHop: true,
//   hopCount: 2,
//   channels: ["channel-6738", "channel-79703"],
//   pathDisplay: "→ channel-6738 → channel-79703 → upenumbra"
// }
```

#### 6. Build an IBC Asset Dashboard

```html
<!DOCTYPE html>
<html>
<head>
  <title>Terp IBC Assets</title>
</head>
<body>
  <h1>IBC Assets on Terp</h1>
  <div id="assets"></div>

  <script>
    async function loadAssets() {
      const lookup = await fetch('/ibc_lookup_table.json').then(r => r.json());
      const terpAssets = lookup.terp || {};
      
      const container = document.getElementById('assets');
      
      for (const [ibcDenom, info] of Object.entries(terpAssets)) {
        const card = document.createElement('div');
        card.className = 'asset-card';
        card.innerHTML = `
          <h3>${info.symbol}</h3>
          <p>Origin: ${info.origin_chain}</p>
          <p>IBC Denom: ${ibcDenom.slice(0, 20)}...</p>
          <p>Hops: ${info.hop_count}</p>
          <p>Trace: ${info.trace_path}</p>
        `;
        container.appendChild(card);
      }
    }

    loadAssets();
  </script>
</body>
</html>
```

### Data Structure Reference for Frontend Developers

#### Simplified Lookup Table (`ibc_lookup_table.json`)

```typescript
interface IBCLookupTable {
  [destChain: string]: {
    [ibcDenom: string]: {
      symbol: string;          // e.g., "AKT"
      origin_chain: string;    // e.g., "akash"
      origin_denom: string;    // e.g., "uakt"
      trace_path: string;      // e.g., "transfer/channel-115/uakt"
      hop_count: number;       // e.g., 1
    }
  }
}
```

#### Full Routing Table (`ibc_routing_table.json`)

```typescript
interface IBCRoutingTable {
  routes: {
    [destChain: string]: IBCAssetRoute[]
  };
  metadata: {
    chains: string[];
    total_routes: number;
    generated_at: string;    // ISO 8601 timestamp
  };
}

interface IBCAssetRoute {
  symbol: string;
  origin_chain: string;
  origin_denom: string;
  dest_chain: string;
  dest_denom: string;        // IBC denom hash
  trace_path: string;
  route: ChannelHop[];
  hop_count: number;
  preferred: boolean;
}

interface ChannelHop {
  from_chain: string;
  to_chain: string;
  from_channel: string;
  to_channel: string;
}
```

### API Design for Static Sites

If serving the lookup table via API:

```
GET /api/ibc/terp/assets          -> All IBC assets on Terp
GET /api/ibc/terp/resolve/:denom  -> Resolve IBC denom to asset info
GET /api/ibc/terp/lookup/:symbol  -> Find IBC denom by symbol
GET /api/ibc/routes/:from/:to     -> Find routes between chains
```

Or embed directly in static HTML:

```html
<script type="application/json" id="ibc-lookup">
  { "terp": { "ibc/...": { ... } } }
</script>
<script>
  const lookup = JSON.parse(document.getElementById('ibc-lookup').textContent);
</script>
```

## Architecture Diagram

```
┌─────────────────────────────────────────────────────────────────┐
│                      Chain Registry Data                         │
│                                                                   │
│  terp/assetlist.json    osmosis/assetlist.json    ...             │
│  _IBC/akash-terp.json   _IBC/osmosis-terp.json   ...             │
└───────────────────────────┬─────────────────────────────────────┘
                            │
                            ▼
┌─────────────────────────────────────────────────────────────────┐
│                    CW-Orch State Manager                          │
│                                                                   │
│  state.json                                                       │
│  ├── morocco-1/assets/      (native + IBC assets)                │
│  ├── morocco-1/ibc_data/   (chain_1/chain_2 format)              │
│  ├── osmosis-1/assets/     (source asset lists)                  │
│  └── ...                                                          │
└───────────────────────────┬─────────────────────────────────────┘
                            │
                            ▼
┌─────────────────────────────────────────────────────────────────┐
│                    IBC Asset Routing Engine                        │
│                                                                   │
│  1. Build IBC Channel Graph from state                           │
│  2. Load asset lists from all chains                             │
│  3. Derive IBC denoms (direct + multi-hop)                      │
│  4. Premine all possible routes                                  │
│  5. Export lookup tables                                         │
└───────────────────────────┬─────────────────────────────────────┘
                            │
                ┌───────────┼───────────┐
                ▼           ▼           ▼
    ┌──────────────┐ ┌──────────────┐ ┌──────────────┐
    │ assetlist.json│ │ ibc_lookup_  │ │ ibc_routing_ │
    │              │ │ table.json   │ │ table.json   │
    │ Full asset   │ │ Simplified   │ │ Full routes  │
    │ list for Terp│ │ O(1) lookup  │ │ with hops    │
    └──────┬───────┘ └──────┬───────┘ └──────┬───────┘
           │                │                │
           ▼                ▼                ▼
    ┌──────────────────────────────────────────────┐
    │           Static Website / Dashboard           │
    │                                                │
    │  - IBC asset explorer                          │
    │  - Portfolio tracker                           │
    │  - Cross-chain swap interface                  │
    │  - IBC denom resolver                          │
    └────────────────────────────────────────────────┘
```

## Testing

### Unit Tests

```bash
# Run all tests
cargo test --test ibc_info

# Run specific test
cargo test --test ibc_info test_akt_direct_channel_to_akash

# Run with output
cargo test --test ibc_info -- --nocapture
```

### Test Coverage

| Test | Description |
|------|-------------|
| `test_compute_ibc_denom_hash_format` | Verifies hash format (ibc/ prefix, 64-char hex) |
| `test_akt_direct_channel_to_akash` | Single-hop: AKT from Akash via channel-115 |
| `test_atone_direct_channel` | Single-hop: ATONE from AtomOne via channel-13 |
| `test_penumbra_via_osmosis` | Multi-hop: UM from Penumbra via Osmosis |
| `test_eth_via_osmosis_then_cosmoshub` | Triple-hop: ETH via Osmosis and Cosmos Hub |
| `test_no_channel_returns_none` | Error case: no available route |
| `test_build_channel_to_chain_map` | Verifies channel map construction from state |

### Integration Test Flow

```
1. Load state.json with IBC data
2. Build channel map
3. Load Osmosis asset list
4. Derive IBC denoms for each asset
5. Verify hashes match expected values
6. Build routing table
7. Export lookup tables
8. Verify JSON output is valid
```

## Extending the Engine

### Adding a New Chain

1. Add the chain's IBC data to `state.json`:
```json
"newchain-terp": {
  "chain_1": { "chain_name": "newchain", ... },
  "chain_2": { "chain_name": "terp", ... },
  "channels": [...]
}
```

2. Add the chain's asset list to state:
```rust
let newchain_assets = load_assetlist_for_chain("newchain", &state_newchain);
chain_assets.insert("newchain".to_string(), newchain_assets);
```

3. Re-run the derivation script. The routing table will automatically include the new chain.

### Adding a New Primitive

The routing table enables several IBC primitives:

**Auto-Relay**: Use the route information to configure Hermes relaying paths.

**Cross-Chain Swap Aggregator**: Find the best route for a token swap across chains.

**Portfolio Tracker**: Resolve all IBC denoms in a wallet to their origin assets.

**Governance Aggregator**: Track governance proposals across chains for IBC-related changes.

**Liquidity Analyzer**: Map which assets are available on which chains and their routes.