# Moved

`cw-private-dex` / **private-dex** now lives in:

```text
crates/terp-rs/contracts/revenue/private-dex
```

Headstash workspace depends on it via:

```toml
cw-private-dex = { package = "private-dex", path = "../terp-rs/contracts/revenue/private-dex" }
```

See `crates/terp-rs/docs/private-bridge/` and `UNIFY-TERP-RS-ECOSYSTEM` (now under docs/private-bridge).
