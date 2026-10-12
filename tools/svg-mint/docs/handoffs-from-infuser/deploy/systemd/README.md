# groot2 user systemd — DREGG + HashMerchant

Units live in `~/.config/systemd/user/` on **groot2**. Linger is on so they start at boot without a login.

```bash
systemctl --user enable --now dregg-svg-mint-fed.service hashmerchant-onboard.service
```

HashMerchant broadcasts with **`terpd-mainnet`** (`ONBOARD_TERPD`), node `tcp://192.168.10.101:26657`, keyring `~/.terp/kr-onboard`. Do not point this at a generic `terpd` binary.

Do not enable `hash-market-lab` docker or house bitcoind.

ZEC tip circuit (optional, fail-closed when set) in `~/.terp/dregg-fed.systemd.env`:

```
ZCASH_PRIVATE_RPC=http://127.0.0.1:8232
ZCASH_PRIVATE_RPC_COOKIE=/home/returniflost/.terp/zakura.rpc.cookie
ZCASH_PUBLIC_RPC=https://zcash-mainnet.gateway.tatum.io/
ZCASH_TIP_MAX_DRIFT=2
```

`ZCASH_PUBLIC_RPC` is **getblockcount only** (not payment watch). Compact streamer stays `ZCASH_DEVTOOL_SERVER=127.0.0.1:9067`. House Zakura cookie-auth is a `user:password` file (`ZCASH_PRIVATE_RPC_COOKIE`). HTTPS public tips use `curl`. Unset public RPC skips the circuit.
