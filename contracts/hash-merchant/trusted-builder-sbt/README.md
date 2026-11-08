# trusted-builder-sbt

Soul-bound token: one mint per Ethereum address that signed the Trustless Manifesto.

No transfer. Build wasm before the ict-rs example:

```sh
# from contract dir (optimizer optional)
cargo build --release --target wasm32-unknown-unknown --lib
```

Example looks for `TRUSTED_BUILDER_WASM` or `artifacts/trusted_builder_sbt.wasm`. Missing file is a **hard error**.
