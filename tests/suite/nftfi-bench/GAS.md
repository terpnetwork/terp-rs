# nftfi gas

Source: in-crate harness in `nftfi-bench` (not the sibling ports).

Mock kv gas uses the raffle `tests/gas.rs` schedule on the in-crate harness: read 1000 + 3 per byte of key and value, write 2000 + 30 per byte, delete 1000. cw-multi-test `gas_used` is 0. The figure is storage gas for the whole transaction, including bank and wasm-keeper writes, not wasmd opcode gas. Marketplace list sums three List executes, loan offer sums two Offer executes, and loan accept sums two Accept executes. Approvals, mints, transfers, and block updates sit outside the window.

Daemon `gas_used` is filled only after a broadcast on svgc-preview-1. This run does not broadcast.

Sibling crates (not path-deps of this bench): revenue-marketplace present, unlinked; revenue-reserve-auction present, unlinked; nftfi-raffle present, unlinked; nftfi-loan present, unlinked.

daemon: skipped. Set NFTFI_BENCH_LOCAL=1 and build with --features local. No transaction sent.

| protocol | action | mock | kv gas | daemon gas |
|---|---|---|---:|---|
| marketplace | list | ok | 50040 | — |
| marketplace | bid | ok | 26893 | — |
| marketplace | buy | ok | 85680 | — |
| marketplace | accept | ok | 69729 | — |
| marketplace | cancel | ok | 6148 | — |
| raffle | create | ok | 48037 | — |
| raffle | buy_ticket | ok | 33031 | — |
| raffle | draw | ok | 23117 | — |
| raffle | claim | ok | 70731 | — |
| loan | offer | ok | 58315 | — |
| loan | accept | ok | 116115 | — |
| loan | repay | ok | 52498 | — |
| loan | default | ok | 27230 | — |
