# HashMerchant onboard: runtime → temp HM → customer

The runtime account must **not** feegrant or authz customers. Live `dao=DAO_ADDR` / `runtime_key=terp1fxcc…` was that collapse. Tear it down; this is the graph.

```text
ONBOARD_RUNTIME          (long-lived)
    MsgGrant GenericAuthorization
    (MsgGrantAllowance + MsgRevokeAllowance only)
        │
        ▼
ONBOARD_HASHMERCHANT     (temp HashMerchant)
    MsgExec → MsgGrantAllowance
    granter = HM, grantee = customer
    AllowedMsgAllowance: AddAuthenticator + RegisterAccount + MsgExecuteContract
    (gas only — never MsgSend / never paying the NFT)
        │
        ▼
customer terp1
```

`WalletAsRuntime` is lab-only (`ONBOARD_ALLOW_COLLAPSE=1`). Production `ready` is false if the two keys are equal or not `terp1`.

HTTP: `POST /onboard/v1/passkey-grant` issues the **one-time** register+mint-gas allowance. `/onboard/v1/mint-grant` stays 404 on nginx (HashMerchant does not buy the NFT).

## Where the mnemonics live (groot2)

Put **only the seed phrases** on **groot2-wan**, under the operator home `~/.terp/`. Do not paste bech32 into chat, git, nginx, or env files — we derive addresses from these files.

| File | Role | Must not be |
|---|---|---|
| `~/.terp/onboard-runtime.mnemonic` | Long-lived **runtime**. Grants authz to HashMerchant only. | Customer `fee.granter`. Not the minter admin file. Not the DREGG mint file. |
| `~/.terp/hashmerchant.mnemonic` | **Temp HashMerchant**. Signs one-time customer feegrants. | The runtime. Not `svg-minter-admin.mnemonic`. Not `dregg-svg-runtime.mnemonic`. |

Create them like this (on groot2, not the laptop):

```bash
umask 077
mkdir -p ~/.terp
nano ~/.terp/onboard-runtime.mnemonic    # 12 or 24 words, one line, no quotes
nano ~/.terp/hashmerchant.mnemonic
chmod 600 ~/.terp/onboard-runtime.mnemonic ~/.terp/hashmerchant.mnemonic
```

Leave these files **alone** (different jobs):

- `~/.terp/dregg-svg-runtime.mnemonic` — DREGG pays native mint
- `~/.terp/svg-minter-admin.mnemonic` — collection/minter admin (`terp1fxcc…`)

When those two new files exist, say so. We derive both `terp1`s, check they differ, then bootstrap authz and point HashMerchant at the derived addresses.

## Authz grant (cw-orch)

This is the existing `hashmerchant_authz_bootstrap` example. It derives both addresses from the mnemonic files, builds `MsgGrant` `GenericAuthorization` for feegrant grant/revoke only, and (with `AUTHZ_BROADCAST=1`) signs as the **runtime** via cw-orch `Daemon.commit_any`.

```bash
# on groot2, after the two mnemonic files exist
cargo run --example hashmerchant_authz_bootstrap -p ict-rs-cw-orch
AUTHZ_BROADCAST=1 cargo run --example hashmerchant_authz_bootstrap -p ict-rs-cw-orch
```

Optional: `TERP_MAIN_GRPC` (default `http://127.0.0.1:9090`). The HashMerchant mnemonic is only used to derive its address — it does not sign this transaction.
