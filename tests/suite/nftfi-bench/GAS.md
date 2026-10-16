# nftfi gas

Source: in-crate harness in `nftfi-bench` (not the sibling ports).

Collateral: cw721-base. Mock cw-orch does not meter wasm gas.

Daemon `gas_used` is filled only after a broadcast on svgc-preview-1. This run does not broadcast.

Sibling crates (not path-deps of this bench): revenue-marketplace present, unlinked; revenue-reserve-auction present, unlinked; nftfi-raffle present, unlinked; nftfi-loan present, unlinked.

daemon: skipped. Set NFTFI_BENCH_LOCAL=1 and build with --features local. No transaction sent.

| protocol | action | mock | detail | daemon gas |
|---|---|---|---|---|
| marketplace | list | ok | unmetered | — |
| marketplace | bid | ok | unmetered | — |
| marketplace | buy | ok | unmetered | — |
| marketplace | accept | ok | unmetered | — |
| marketplace | cancel | ok | unmetered | — |
| raffle | create | ok | unmetered | — |
| raffle | buy_ticket | ok | unmetered | — |
| raffle | draw | ok | unmetered | — |
| raffle | claim | ok | unmetered | — |
| loan | offer | ok | unmetered | — |
| loan | accept | ok | unmetered | — |
| loan | repay | ok | unmetered | — |
| loan | default | ok | unmetered | — |
