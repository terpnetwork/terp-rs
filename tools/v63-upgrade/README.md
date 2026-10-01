# v6.3 expedited proposal

Signs one `x/gov` expedited `MsgSoftwareUpgrade` for plan `v6.3` on `morocco-1`.
The v6.3 binary arms `v6.4`. This binary does not submit `v6.4` and does not
call the DAO.

Mnemonic file (gitignored). Fill `MAIN_MNEMONIC` by hand:

`/Users/returniflost/abstract/terp-core/crates/terp-rs/.env.v63-upgrade`

Dry-run:

```sh
cargo run --manifest-path crates/terp-rs/tools/v63-upgrade/Cargo.toml --release
```

Broadcast requires `UPGRADE_BROADCAST=1` in that file, a non-empty mnemonic,
and an empty voting and deposit queue. Run from the terp-core checkout so the
draft at `networks/upgrades/v6.3/draft_proposal.json` resolves.
