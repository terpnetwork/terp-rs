# Headstash Dashboard

# egui headstash : front end tools

- dedicated wasm-bindgen api library: complete,reusable,lightweight wasm-bindgen crate powering headstash note-nullifier data preparation & proof generation via web communication through wasm-bindgeen.
- snap-n-pull: Metamask-snap plugin powered by wasm-bindgen crate

## Front End DashBoard: Headstash

- **layer-climb-core**: Full-featured QueryClient with middleware system already implemented
- **AppClient**: Wraps QueryClient with network management and gRPC/REST fallback
- **Smart Account Authentication**: Fully implemented with ETH offline signer integration
- **Chain Registry**: Network configuration management with multi-environment support
- **Wallet Integration**: Complete wallet connection and transaction signing capabilities
  - retrieve data from & and snap cosmos wallet window to ront for seamless experience for wallet use between windows
  - native account offline signing: metamask, keplr phantom, ledger, penumbra wallet, others
  - js-bindgen for app comms with wallet in windows.
  - switch for smart-account authentication use: implement support for defining dedicated smart account id and specification injection for signing actions.

## Requirements Clarification

### What We're Building

1. **reusable Window Components egui:**

- **Easy-to-use egui macros and traits for proof/chain/vm client UIs** macro derived components: modular middleware, indexer,authentication,client defintions per window, for modular definition of new windows we want to wrap into access of global app layer integrated with wallets wasm-bingen statefulness,  modular wallet-powered window support
  - drect smart contract ypes will be used for queries entrypoints: because we are in rust, we will use smart contract query definitions encoded directly to vec for protobuf serialization support, for queries and actions.

1. **Smart Account Authentication Manager**

- register,manage,use authentication via smart_account panel
- view authenticators: use smart account service to query connected wallets registered accounts for authentication
- integration for template authenticators
- visualize authenticator widget appropriately with respect to their recursive definition: `AllOf`,`AnyOf`,`AnyOfBlend` are able to be configured for authenticators, so front end should register/visualize this when registreing authetnicators in a intuitive manner so that we can depyct the layers of authenitcation in a neat manner.  this should eb done in th emodules where th parameter forms will be for each authenticator being added, and also as a review chart underneath for viusal ques of confirmation of layerout and structure.

1. 1-click decentralization support:

2. **Zk-Headstash** Airdrop Distribution Portal

## egui: web-control panel

### Cosmos Clients Specification: Layer-Climb

### Chain & Indexer Config

- define chain & indexerconfigs statically
- prioritize indexer query, fallback manual chain rpc via middleware support
- canonical template for defining knwon chains, contracts, app-frameworks

### Headstash Marketplace

### Template Macro Defintions

### Sovereign Authentication And Custody Support

#### Authenticator Panel

### metamask snaps: metamask plugin

- generates hash to curve for pallas field & proof in metamask wallet
- simple install for metamask snaps, publicly accountably trustless code

### development libraries: js,rust,python

# Web Wallet Integration Specification

## egui + WASM + Browser Wallet Extensions

**Version:** 1.0
**Last Updated:** 2025-01-19
**Status:** Production-Ready (Connect, Disconnect, View Account Data)

---

## Overview

This specification defines the complete architecture for integrating browser-based cryptocurrency wallets (Keplr, MetaMask) with egui applications compiled to WebAssembly. The system provides a reactive, stateful wallet connection interface that works seamlessly in web browsers.

### Supported Wallets

| Wallet | Chain Type | Status | Capabilities |
|--------|-----------|--------|--------------|
| **Keplr** | Cosmos/Terp | ✅ Production | Connect, Disconnect, View Account, Chain Selection |
| **MetaMask** | Ethereum | ✅ Production | Connect, Disconnect, View Account |

### Current Capabilities

✅ **Wallet Detection** - Automatic detection of installed browser extensions
✅ **Connection Flow** - User-initiated connection with approval prompt
✅ **Disconnect Flow** - Clean disconnection with state cleanup
✅ **Account Display** - Address and public key display
✅ **Chain Selection** - Multi-chain support with registry
✅ **Reactive UI** - Real-time state updates with proper repaint triggers
✅ **Error Handling** - User-friendly error messages and recovery
✅ **Logging** - Comprehensive debug logging across JS/Rust boundary

---

## Architecture Overview

### Component Layers

```
┌─────────────────────────────────────────────────────────────────┐
│                         User Interface                          │
│  egui Windows (CosmosWindow) → WalletConnectionComponent       │
└────────────────────────┬────────────────────────────────────────┘
                         │
┌────────────────────────┴────────────────────────────────────────┐
│                    State Management Layer                       │
│           WalletConnectionManager (State Machine)               │
│  States: Disconnected → Connecting → Querying → Active         │
└────────────────────────┬────────────────────────────────────────┘
                         │
┌────────────────────────┴────────────────────────────────────────┐
│                   WASM Bridge Layer (Rust)                      │
│           wallet/bridge.rs (wasm-bindgen bindings)              │
│  Functions: connect::keplr(), connect::metamask()               │
└────────────────────────┬────────────────────────────────────────┘
                         │
                    JS Boundary
                         │
┌────────────────────────┴────────────────────────────────────────┐
│              JavaScript Bridge (Browser Context)                │
│          static/wallet_bridge.js (window functions)             │
│  Functions: requestKeplrEnable(), getKeplrAccount(), etc.       │
└────────────────────────┬────────────────────────────────────────┘
                         │
┌────────────────────────┴────────────────────────────────────────┐
│                   Browser Wallet Extensions                     │
│           window.keplr, window.ethereum (injected)              │
└─────────────────────────────────────────────────────────────────┘
```

---

## Core Components

### 1. JavaScript Bridge (`static/wallet_bridge.js`)

**Purpose:** Provides window-scoped functions for WASM to call browser wallet APIs.

**Key Functions:**

```javascript
// Wallet Detection
window.isKeplrInstalled() → boolean
window.isMetaMaskInstalled() → boolean

// Keplr Wallet
window.requestKeplrEnable(chainId: string) → Promise<void>
window.getKeplrAccount(chainId: string) → Promise<KeplrKey>
window.signKeplrAmino(chainId, signer, signDoc) → Promise<AminoResponse>

// MetaMask Wallet
window.requestMetaMaskAccounts() → Promise<string[]>
window.getMetaMaskAccounts() → Promise<string[]>
window.signMetaMaskPersonal(message, address) → Promise<string>
window.signMetaMaskTypedData(address, typedData) → Promise<string>

// Event Listeners
window.setupKeplrListeners(onKeystoreChange)
window.setupMetaMaskListeners(onAccountsChanged, onChainChanged, onDisconnect)
```

**Design Principles:**

- All functions attached to `window` object (not ES6 modules)
- Compatible with `--target no-modules` WASM build flag
- Comprehensive `[BRIDGE]` prefixed logging for debugging
- Explicit error handling with descriptive messages
- Promise-based async interface

**Example:**

```javascript
window.requestKeplrEnable = async function(chainId) {
    console.log('[BRIDGE] requestKeplrEnable called for chain:', chainId);

    if (!window.keplr) {
        console.error('[BRIDGE] Keplr not found on window object');
        throw new Error('Keplr extension is not installed');
    }

    try {
        console.log('[BRIDGE] Calling window.keplr.enable()...');
        await window.keplr.enable(chainId);
        console.log('[BRIDGE] Keplr enable() completed successfully');
        return;
    } catch (error) {
        console.error('[BRIDGE] Failed to enable Keplr for chain', chainId, error);
        throw error;
    }
}
```

### 2. Rust Bridge (`wallet/bridge.rs`)

**Purpose:** wasm-bindgen declarations and high-level connection logic.

**Structure:**

```rust
#[cfg(target_arch = "wasm32")]
#[wasm_bindgen]
extern "C" {
    /// Declarations of JS functions from wallet_bridge.js
    #[wasm_bindgen(js_namespace = window)]
    pub fn isKeplrInstalled() -> bool;

    #[wasm_bindgen(js_namespace = window, catch)]
    pub async fn requestKeplrEnable(chain_id: &str) -> Result<JsValue, JsValue>;

    #[wasm_bindgen(js_namespace = window, catch)]
    pub async fn getKeplrAccount(chain_id: &str) -> Result<JsValue, JsValue>;

    // ... more declarations
}

/// High-level connection functions
pub mod connect {
    /// Connect to Keplr wallet
    pub async fn keplr(chain_id: &str) -> Result<WalletConnection, String> {
        log::info!("[BRIDGE_RS] Starting Keplr connection for chain: {}", chain_id);

        if !is_keplr_installed() {
            return Err("Keplr wallet is not installed".to_string());
        }

        // Enable chain
        request_keplr_enable(chain_id).await?;

        // Get account
        let account_js = get_keplr_account(chain_id).await?;
        let key: KeplrKey = serde_wasm_bindgen::from_value(account_js)?;

        Ok(WalletConnection {
            address: key.bech32_address,
            public_key: Some(key.pubkey),
            wallet_type: WalletType::Keplr,
        })
    }

    /// Connect to MetaMask wallet
    pub async fn metamask() -> Result<WalletConnection, String> { /* ... */ }
}
```

**Key Types:**

```rust
pub struct WalletConnection {
    pub address: String,
    pub public_key: Option<Vec<u8>>,
    pub wallet_type: WalletType,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WalletType {
    Keplr,
    MetaMask,
    Direct,  // Future: mnemonic-based
}
```

### 3. Connection Manager (`wallet/connection_manager.rs`)

**Purpose:** State machine managing wallet connection lifecycle.

**State Machine:**

```rust
#[derive(Debug, Clone, PartialEq)]
pub enum ConnectionState {
    Disconnected,      // No wallet connected
    Detecting,         // Checking wallet availability
    Connecting,        // User approving in wallet popup
    Querying,          // Fetching initial account data
    Active,            // Fully connected (WASM-compatible, no Instant)
    Error { message: String, retry_count: u32 },
    Disconnecting,     // Cleanup in progress
}

impl ConnectionState {
    pub fn is_connected(&self) -> bool {
        matches!(self, ConnectionState::Active)
    }

    pub fn is_transitioning(&self) -> bool {
        matches!(
            self,
            ConnectionState::Detecting
                | ConnectionState::Connecting
                | ConnectionState::Querying
                | ConnectionState::Disconnecting
        )
    }
}
```

**Key Methods:**

```rust
impl WalletConnectionManager {
    /// Start connection flow
    pub async fn connect(
        &self,
        wallet_type: WalletType,
        chain_config: ChainConfig,
    ) -> Result<(), String> {
        // 1. Update state to Connecting
        self.update_state(ConnectionState::Connecting);

        // 2. Call bridge function (keplr or metamask)
        let connection = match wallet_type {
            WalletType::Keplr => connect::keplr(&chain_config.chain_id).await?,
            WalletType::MetaMask => connect::metamask().await?,
            _ => return Err("Unsupported wallet type".to_string()),
        };

        // 3. Update state to Querying
        self.update_state(ConnectionState::Querying);

        // 4. Store account info
        let account = AccountInfo {
            address: connection.address,
            public_key: connection.public_key,
            balance: None,
            sequence: None,
            account_number: None,
        };
        self.update_account(account);

        // 5. Set active state
        self.update_state(ConnectionState::Active);

        // 6. Start background polling
        self.start_polling();

        Ok(())
    }

    /// Disconnect wallet
    pub async fn disconnect(&self) -> Result<(), String> {
        self.update_state(ConnectionState::Disconnecting);
        self.clear_account();
        self.update_state(ConnectionState::Disconnected);
        Ok(())
    }

    /// Refresh account data (stub for now)
    pub async fn refresh(&self) -> Result<(), String> {
        // TODO: Query balance using layer-climb QueryClient
        Ok(())
    }
}
```

**Account Info:**

```rust
#[derive(Debug, Clone)]
pub struct AccountInfo {
    pub address: String,
    pub public_key: Option<Vec<u8>>,
    pub balance: Option<Vec<Coin>>,
    pub sequence: Option<u64>,
    pub account_number: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct Coin {
    pub denom: String,
    pub amount: String,
}
```

### 4. UI Component (`components/wallet.rs`)

**Purpose:** Reactive egui component rendering wallet connection UI.

**Structure:**

```rust
pub struct WalletConnectionComponent {
    connection_manager: WalletConnectionManager,
    chain_registry: ChainRegistry,
    selected_chain_id: Option<String>,
    selected_wallet: Option<WalletType>,
    refreshing: bool,
}

impl WalletConnectionComponent {
    /// Public API for external integration
    pub fn is_connected(&self) -> bool { /* ... */ }
    pub fn get_address(&self) -> Option<String> { /* ... */ }
    pub fn get_state(&self) -> ConnectionState { /* ... */ }
    pub fn get_chain_config(&self) -> Option<ChainConfig> { /* ... */ }

    /// Main UI entry point
    pub fn ui(&mut self, ui: &mut Ui) {
        // Request repaint during transitions
        if self.connection_manager.get_state().is_transitioning() {
            ui.ctx().request_repaint();
        }

        match self.connection_manager.get_state() {
            ConnectionState::Disconnected => self.render_disconnected(ui),
            ConnectionState::Connecting => self.render_connecting(ui),
            ConnectionState::Active => self.render_active(ui),
            ConnectionState::Error { .. } => self.render_error(ui),
            // ...
        }
    }
}
```

**Critical: egui Repaint Triggers**

```rust
#[cfg(target_arch = "wasm32")]
fn initiate_connection(&mut self, ctx: &egui::Context) {
    use wasm_bindgen_futures::spawn_local;

    let manager = self.connection_manager.clone();
    let ctx = ctx.clone();  // Clone for async task

    spawn_local(async move {
        match manager.connect(wallet_type, chain_config).await {
            Ok(()) => {
                log::info!("Connection successful!");
                ctx.request_repaint();  // ⚠️ CRITICAL for UI update
            }
            Err(e) => {
                log::error!("Connection failed: {}", e);
                manager.set_error(e);
                ctx.request_repaint();  // ⚠️ CRITICAL for UI update
            }
        }
    });
}
```

**Why `request_repaint()` is Required:**

egui is an **immediate mode** GUI framework. When async tasks complete in `spawn_local`, they run outside the normal render loop. Without explicit `ctx.request_repaint()`, the UI won't update until the next user interaction (mouse move, click, etc.), causing the "stuck on Connecting" issue.

### 5. Chain Registry (`chain_registry.rs`)

**Purpose:** Type-safe chain configuration management.

**Structure:**

```rust
#[derive(Debug, Clone, PartialEq)]
pub struct ChainConfig {
    pub id: String,
    pub name: String,
    pub chain_id: String,
    pub rpc_endpoint: String,
    pub rest_endpoint: String,
    pub grpc_web_endpoint: Option<String>,
    pub gas_price: f64,
    pub gas_denom: String,
    pub bech32_prefix: String,
    pub coin_type: u32,
    pub explorer_url: Option<String>,
    pub features: Vec<String>,
}

pub struct ChainRegistry {
    chains: HashMap<String, ChainConfig>,
}

impl ChainRegistry {
    /// Load from static/config.json
    pub fn new() -> Self { /* ... */ }

    /// Get chain by ID
    pub fn get(&self, id: &str) -> Option<&ChainConfig> { /* ... */ }

    /// Get all testnets
    pub fn testnets(&self) -> Vec<&ChainConfig> { /* ... */ }

    /// Get all mainnets
    pub fn mainnets(&self) -> Vec<&ChainConfig> { /* ... */ }

    /// Add custom chain at runtime
    pub fn add_chain(&mut self, config: ChainConfig) -> Result<(), String> { /* ... */ }
}
```

---

## Data Flow

### Connection Flow

```
1. User Action
   ├─ User selects chain from dropdown
   ├─ User clicks "Keplr" button
   └─ initiate_connection() called

2. State Transition: Disconnected → Connecting
   ├─ UI shows spinner + "Connecting..." message
   └─ Cancel button available

3. Async Task Spawned (spawn_local)
   ├─ manager.connect(WalletType::Keplr, chain_config)
   └─ Runs in background

4. WASM → JS Bridge Call
   ├─ connect::keplr(chain_id)
   ├─ request_keplr_enable(chain_id)
   └─ Calls window.requestKeplrEnable()

5. JavaScript → Browser Extension
   ├─ window.keplr.enable(chain_id)
   └─ Keplr popup appears

6. User Approval in Wallet
   ├─ User clicks "Approve" in Keplr popup
   └─ Promise resolves

7. JS → WASM: Account Data
   ├─ window.keplr.getKey(chain_id)
   ├─ Returns KeplrKey object
   └─ Serialized via serde_wasm_bindgen

8. State Transition: Connecting → Querying
   ├─ Account info stored
   └─ public_key, address extracted

9. State Transition: Querying → Active
   ├─ Connection complete
   └─ Background polling started

10. UI Update Trigger
    ├─ ctx.request_repaint() called
    └─ UI re-renders with "Connected" state

11. UI Shows Connected State
    ├─ Green "● Connected" indicator
    ├─ Wallet address displayed
    ├─ Disconnect button available
    └─ Balance queries begin (background)
```

### Disconnect Flow

```
1. User Action
   └─ User clicks "Disconnect" button

2. Async Disconnect
   ├─ manager.disconnect()
   └─ State: Active → Disconnecting

3. Cleanup
   ├─ Stop background polling
   ├─ Clear account info
   └─ Clear wallet type and chain config

4. State Transition
   └─ Disconnecting → Disconnected

5. UI Update
   ├─ ctx.request_repaint() called
   └─ UI shows "● Disconnected"
```

---

## Configuration

### HTML Setup (`web_demo/index.html`)

```html
<!DOCTYPE html>
<html>
<head>
    <meta charset="utf-8">
    <title>egui Cosmos Demo</title>
</head>
<body>
    <!-- Wallet bridge MUST load before WASM -->
    <script src="./wallet_bridge.js"></script>

    <!-- Configuration -->
    <script src="./config.json" type="application/json"></script>

    <!-- WASM app -->
    <script src="./egui_demo_app.js"></script>
</body>
</html>
```

⚠️ **Critical:** `wallet_bridge.js` must load before the WASM module initializes, otherwise `window.isKeplrInstalled()` will be undefined.

### Build Script (`scripts/build_demo_web.sh`)

```bash
#!/bin/bash
set -eu

# Build WASM
./scripts/build_web.sh

# Copy static files to web_demo/
echo "Copying wallet bridge and config files…"
cp -f crates/egui_demo_lib/src/cosmos/static/wallet_bridge.js web_demo/
cp -f crates/egui_demo_lib/src/cosmos/static/config.json web_demo/

echo "Build complete! Run ./scripts/start_server.sh to test"
```

### Chain Configuration (`static/config.json`)

```json
{
  "chains": [
    {
      "id": "terp-testnet",
      "name": "Terp Testnet",
      "chain_id": "morocco-1",
      "rpc_endpoint": "https://rpc-testnet.terp.network",
      "rest_endpoint": "https://api-testnet.terp.network",
      "grpc_web_endpoint": "https://grpc-web-testnet.terp.network",
      "gas_price": 0.025,
      "gas_denom": "uterpx",
      "bech32_prefix": "terp",
      "coin_type": 118,
      "explorer_url": "https://explorer-testnet.terp.network",
      "features": ["smartaccount", "fantoken"]
    }
  ]
}
```

---

## Error Handling

### Error Types and Recovery

| Error | Cause | User Message | Recovery |
|-------|-------|--------------|----------|
| `Wallet not installed` | Extension not detected | "Keplr is not installed" | Show install link |
| `User rejected` | User clicked "Reject" | "Connection rejected by user" | Retry button |
| `Chain not found` | Invalid chain_id | "Chain not configured" | Check config.json |
| `Enable failed` | keplr.enable() threw | "Failed to enable chain: {error}" | Retry or add chain |
| `Account fetch failed` | getKey() threw | "Failed to get account: {error}" | Retry button |
| `Parse error` | Invalid JS response | "Failed to parse account data" | Debug bridge.js |

### Error State Display

```rust
fn render_error(&mut self, ui: &mut Ui) {
    ui.horizontal(|ui| {
        ui.label(RichText::new("●").color(Color32::RED).size(20.0));
        ui.label(RichText::new("Connection Error").strong().color(Color32::RED));
    });

    if let Some(error) = self.connection_manager.get_error() {
        ui.group(|ui| {
            ui.colored_label(Color32::RED, &error);
        });
    }

    ui.horizontal(|ui| {
        if ui.button("🔄 Retry").clicked() {
            self.connection_manager.clear_error();
            self.initiate_connection(ui.ctx());
        }

        if ui.button("Cancel").clicked() {
            self.disconnect(ui.ctx());
        }
    });
}
```

---

## Debugging

### Log Sequence (Successful Keplr Connection)

```
[INFO] Starting connection to Keplr
[INFO] Connection manager: Starting connection to Keplr
[INFO] Connection manager: State updated to Connecting
[BRIDGE_RS] Starting Keplr connection for chain: morocco-1
[BRIDGE_RS] Keplr detected
[BRIDGE_RS] Requesting Keplr enable for chain...
[BRIDGE] requestKeplrEnable called for chain: morocco-1
[BRIDGE] Calling window.keplr.enable()...

// ⏳ User approves in Keplr popup

[BRIDGE] Keplr enable() completed successfully
[BRIDGE_RS] Keplr enable() completed
[BRIDGE_RS] Requesting account info...
[BRIDGE] getKeplrAccount called for chain: morocco-1
[BRIDGE] Calling window.keplr.getKey()...
[BRIDGE] Keplr getKey() returned: {name: "Account 1", algo: "secp256k1", ...}
[BRIDGE] Returning account: terp1xxx...
[BRIDGE_RS] Received account data from JS
[BRIDGE_RS] Parsing account data...
[BRIDGE_RS] Successfully parsed account: terp1xxx...
[INFO] Connection manager: Wallet approved! Address: terp1xxx...
[INFO] Connection manager: State updated to Querying
[INFO] Connection manager: Account info stored
[INFO] Connection manager: State updated to Active
[INFO] Connection successful!

// ✅ UI updates to "● Connected"
```

### Browser DevTools Checklist

**Console Tab:**

- [ ] No JavaScript errors on page load
- [ ] `wallet_bridge.js` loaded successfully
- [ ] `window.isKeplrInstalled()` returns `true` (if Keplr installed)
- [ ] Full log sequence appears during connection
- [ ] `[BRIDGE]` and `[BRIDGE_RS]` logs interleaved correctly

**Network Tab:**

- [ ] `wallet_bridge.js` - 200 OK
- [ ] `config.json` - 200 OK
- [ ] `egui_demo_app.js` - 200 OK
- [ ] `egui_demo_app.wasm` - 200 OK

**Application Tab:**

- [ ] No CORS errors
- [ ] Service worker not interfering (if testing locally)

### Common Issues

**Issue:** UI stuck on "Connecting..."

**Cause:** Missing `ctx.request_repaint()` in async callback

**Fix:** Ensure all `spawn_local` async blocks call `ctx.request_repaint()` on completion

---

**Issue:** `panicked at time not implemented on this platform`

**Cause:** Using `std::time::Instant` in WASM (not supported)

**Fix:** Remove `Instant` fields from `ConnectionState::Active` or use `web-time` crate

---

**Issue:** `isKeplrInstalled is not a function`

**Cause:** `wallet_bridge.js` not loaded or loaded after WASM

**Fix:** Ensure `<script src="./wallet_bridge.js"></script>` comes before `egui_demo_app.js`

---

**Issue:** Wallet shows connected but UI shows disconnected

**Cause:** State desync, likely from hot reload or page refresh

**Fix:** Full page refresh, check wallet extension shows correct dapp connection

---

## Integration Example

### Adding Wallet Connection to Your egui Window

```rust
use crate::cosmos::components::WalletConnectionComponent;

pub struct MyWindow {
    wallet_connection: WalletConnectionComponent,
}

impl Default for MyWindow {
    fn default() -> Self {
        Self {
            wallet_connection: WalletConnectionComponent::new(),
        }
    }
}

impl MyWindow {
    pub fn ui(&mut self, ui: &mut egui::Ui) {
        // Render wallet connection UI
        self.wallet_connection.ui(ui);

        ui.separator();

        // Only show other features if connected
        if self.wallet_connection.is_connected() {
            if let Some(address) = self.wallet_connection.get_address() {
                ui.heading("Your Account");
                ui.monospace(&address);

                // Access chain config
                if let Some(chain) = self.wallet_connection.get_chain_config() {
                    ui.label(format!("Chain: {}", chain.name));
                    ui.label(format!("RPC: {}", chain.rpc_endpoint));
                }
            }
        } else {
            ui.label("Connect wallet to continue");
        }
    }
}
```

---

## WASM Compatibility Notes

### ⚠️ Platform-Specific Issues

1. **Time APIs:** `std::time::Instant` **NOT** supported in WASM
   - ✅ Use: Simple state flags, frame counters
   - ❌ Avoid: `Instant::now()`, `Duration::elapsed()`

2. **Threading:** `std::thread` **NOT** supported
   - ✅ Use: `spawn_local` for async tasks
   - ❌ Avoid: `thread::spawn`, `std::sync::mpsc`

3. **File I/O:** Direct file system access **NOT** supported
   - ✅ Use: Embedded assets via `include_str!()`, web fetch APIs
   - ❌ Avoid: `std::fs::read`, `File::open`

4. **Module System:** ES6 modules require `--target web` flag
   - ✅ Use: `window` object functions with `--target no-modules`
   - ❌ Avoid: ES6 `export`/`import` with `--target no-modules`

### Cargo.toml Features

```toml
[target.'cfg(target_arch = "wasm32")'.dependencies]
wasm-bindgen = "0.2"
wasm-bindgen-futures = "0.4"
web-sys = { version = "0.3", features = ["Window", "Navigator"] }
serde-wasm-bindgen = "0.6"

[dependencies]
# Ensure tokio doesn't pull in unsupported features
tokio = { version = "1.47", default-features = false }
```

---

## Future Enhancements

### Phase 2: Transaction Signing

```rust
pub async fn sign_transaction(
    &self,
    msgs: Vec<Any>,
    fee: Fee,
    memo: String,
) -> Result<TxRaw, String> {
    let wallet_type = self.get_wallet_type()
        .ok_or("No wallet connected")?;

    match wallet_type {
        WalletType::Keplr => {
            // Use signKeplrAmino from bridge
            let sign_doc = create_amino_sign_doc(msgs, fee, memo)?;
            let signature = sign_keplr_amino(chain_id, signer, sign_doc).await?;
            Ok(build_tx_raw(signature)?)
        }
        WalletType::MetaMask => {
            Err("MetaMask signing not yet implemented".to_string())
        }
        _ => Err("Unsupported wallet type".to_string()),
    }
}
```

### Phase 3: Balance Queries (layer-climb Integration)

```rust
pub async fn query_balance(
    &self,
    address: &str,
    denom: &str,
) -> Result<Coin, String> {
    use layer_climb::prelude::*;

    let chain = self.get_chain_config()
        .ok_or("No chain configured")?;

    let grpc_web = chain.grpc_web_endpoint
        .ok_or("Chain missing gRPC-web endpoint")?;

    let client = QueryClient::new(&grpc_web).await?;
    let balance = client.bank_balance(address, denom).await?;

    Ok(Coin {
        denom: balance.denom,
        amount: balance.amount,
    })
}
```

### Phase 4: Multi-Wallet Support

Allow connecting both Keplr and MetaMask simultaneously:

```rust
pub struct MultiWalletManager {
    keplr_connection: Option<WalletConnectionManager>,
    metamask_connection: Option<WalletConnectionManager>,
}

impl MultiWalletManager {
    pub fn connect_keplr(&mut self, chain: ChainConfig) { /* ... */ }
    pub fn connect_metamask(&mut self) { /* ... */ }
    pub fn get_primary_signer(&self) -> Option<&WalletConnectionManager> { /* ... */ }
}
```

### Phase 5: Local Test Wallet

Browser localStorage-based test wallet for development:

```rust
pub async fn generate_test_wallet() -> Result<WalletConnection, String> {
    let mnemonic = bip39::Mnemonic::generate(24)?;
    let seed = mnemonic.to_seed("");
    let key = derive_cosmos_key(seed, 118, 0, 0)?;

    // Store in localStorage
    store_encrypted_mnemonic(&mnemonic, password)?;

    Ok(WalletConnection {
        address: key.address,
        public_key: Some(key.pubkey),
        wallet_type: WalletType::Direct,
    })
}
```

---

## Testing Checklist

### Manual Testing

- [ ] **Build:** `./scripts/build_demo_web.sh` completes without errors
- [ ] **Serve:** `./scripts/start_server.sh` starts local server
- [ ] **Load:** Open `http://localhost:8765/index.html` with no console errors
- [ ] **Detect:** Keplr button shows enabled (if installed)
- [ ] **Select:** Chain selector shows all configured chains
- [ ] **Connect:** Click Keplr → popup appears
- [ ] **Approve:** Approve in Keplr → UI updates to "● Connected" (green)
- [ ] **Display:** Address shows correctly
- [ ] **Disconnect:** Click disconnect → UI updates to "● Disconnected"
- [ ] **Reconnect:** Can reconnect successfully
- [ ] **Error:** Reject connection → shows error state with retry
- [ ] **MetaMask:** Same flow works for MetaMask

### Automated Testing (Future)

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use wasm_bindgen_test::*;

    #[wasm_bindgen_test]
    async fn test_keplr_connection() {
        let manager = WalletConnectionManager::new();
        let config = ChainConfig::testnet();

        let result = manager.connect(WalletType::Keplr, config).await;
        assert!(result.is_ok());
        assert!(manager.get_state().is_connected());
    }
}
```

---

## API Reference Summary

### JavaScript Bridge API

| Function | Parameters | Returns | Description |
|----------|-----------|---------|-------------|
| `isKeplrInstalled()` | - | `boolean` | Check if Keplr installed |
| `isMetaMaskInstalled()` | - | `boolean` | Check if MetaMask installed |
| `requestKeplrEnable(chainId)` | `string` | `Promise<void>` | Enable Keplr for chain |
| `getKeplrAccount(chainId)` | `string` | `Promise<KeplrKey>` | Get Keplr account info |
| `requestMetaMaskAccounts()` | - | `Promise<string[]>` | Request MetaMask accounts |

### Rust Public API

| Method | Description |
|--------|-------------|
| `WalletConnectionComponent::new()` | Create component |
| `is_connected()` | Check if wallet connected |
| `get_address()` | Get connected address |
| `get_state()` | Get current connection state |
| `ui(&mut self, ui)` | Render UI |

### State Transitions

```
Disconnected → Connecting → Querying → Active
     ↑            ↓                        ↓
     └────────────┴────── Error ──────────┘
```

---

## File Structure

```
crates/egui_demo_lib/src/cosmos/
├── static/
│   ├── wallet_bridge.js      # JS bridge functions
│   └── config.json            # Chain configurations
├── wallet/
│   ├── mod.rs
│   ├── bridge.rs              # wasm-bindgen declarations
│   └── connection_manager.rs  # State machine
├── components/
│   ├── mod.rs
│   └── wallet.rs              # UI component
├── chain_registry.rs          # Chain config management
└── window.rs                  # Main Cosmos window

web_demo/
├── index.html                 # Entry point (loads wallet_bridge.js)
├── wallet_bridge.js           # Copied from static/
└── config.json                # Copied from static/

scripts/
├── build_demo_web.sh          # Build + copy static files
└── start_server.sh            # Local development server
```

---

## Conclusion

This specification documents a **production-ready** wallet integration system for egui + WASM applications. The architecture is:

✅ **Modular** - Clean separation between JS bridge, state management, and UI
✅ **Type-Safe** - Rust type system enforces correctness
✅ **Reactive** - UI updates automatically via proper repaint triggers
✅ **Debuggable** - Comprehensive logging across JS/Rust boundary
✅ **Extensible** - Easy to add new wallets, chains, and features

**Current Status:** Connect, Disconnect, View Account Data (Keplr + MetaMask)

**Next Steps:** Transaction signing, balance queries, multi-wallet support

---

**Version History:**

- **v1.0** (2025-01-19) - Initial production specification after fixing WASM `Instant` issue and repaint triggers
