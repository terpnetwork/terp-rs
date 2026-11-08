# v6.3 expedited proposal

Signs one `x/gov` expedited `MsgSoftwareUpgrade` for plan `v6.3` on `morocco-1`.
The v6.3 binary arms `v6.4`. This binary does not submit `v6.4` and does not
call the DAO.

Mnemonic file (gitignored), only when broadcasting. Fill `MAIN_MNEMONIC` by hand:

`/Users/returniflost/abstract/terp-core/crates/terp-rs/.env.v63-upgrade`

Dry-run reads the upgrade files and signs nothing. The env file is optional
until broadcast:

```sh
cargo run --manifest-path crates/terp-rs/tools/v63-upgrade/Cargo.toml --release
```

Broadcast requires `UPGRADE_BROADCAST=1` in that file and a non-empty
mnemonic. Height, deposit, checksums, and the v6.3.0 / v6.4.0 commits come
from `networks/upgrades/v6.3/` (`draft_proposal.json`, `proposal.json`,
`metadata.json`, `cosmovisor.json`, `binaries.json`) and
`networks/upgrades/v6.4/cosmovisor.json` with its matching `binaries.json`.
Those paths are resolved from this crate, not from the shell's working
directory. An open text or deposit proposal does not block the submit. An
open software upgrade does. The halt must be at least 28800 blocks ahead of
the LCD at `LCD` (default `http://192.168.10.101:1317`). Gas is `uthiol`.
