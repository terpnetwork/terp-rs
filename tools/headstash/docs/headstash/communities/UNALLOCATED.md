# Headstash unallocated (burn) — EVM communities

Original snapshot CSVs (`addr,amount`) are **not** modified.

Rows without a recoverable secp256k1 pubkey cannot receive a Headstash note.
Their snapshot **amount** is documented here to **burn** rather than allocate.
Amounts are **per-community snapshot units** (NFT counts vs token balances) — do not sum across rows as one denom.

| Class | Meaning |
|-------|--------|
| `dead:contract` | `eth_getCode` nonempty |
| `dead:never-sent` | mainnet nonce 0 |
| `dead:recover-mismatch` | recovered signer ≠ holder |
| `blind:pending` | explorer/RPC did not return a signed tx this scrape |
| `blind:no-signed-tx` | confirmed no outgoing signed tx |
| `blind:recover-failed` | tx present but signature recover failed |

| Community | keyed | unallocated rows | unallocated amount | dead amount | blind amount |
|-----------|------:|-----------------:|-------------------:|------------:|-------------:|
| buddah-bears | 2278/2453 | 175 | 636.0000 | 636.0000 | 0.0000 |
| cannabuddies | 215/238 | 23 | 39.0000 | 39.0000 | 0.0000 |
| carta-beta-gang | 55/157 | 102 | 111.0000 | 111.0000 | 0.0000 |
| chronic-token | 1176/1255 | 79 | 28022902.6300 | 28022902.6300 | 0.0000 |
| crypto-canna-club | 4109/4405 | 296 | 619.0000 | 619.0000 | 0.0000 |
| cryptowizards | 54/55 | 1 | 1.0000 | 1.0000 | 0.0000 |
| galacktic-gang | 2440/2568 | 128 | 295.0000 | 295.0000 | 0.0000 |
| heady-pipe-society | 22/23 | 1 | 1.0000 | 1.0000 | 0.0000 |
| hippie-life-krew | 312/342 | 30 | 119.0000 | 119.0000 | 0.0000 |
| monster-buds | 3160/3385 | 225 | 895.0000 | 895.0000 | 0.0000 |
| rebud | 570/585 | 15 | 43.0000 | 43.0000 | 0.0000 |
| secret-sesh | 471/527 | 56 | 132.0000 | 132.0000 | 0.0000 |
| shurlok | 23/33 | 10 | 15.0000 | 15.0000 | 0.0000 |
| special-k | 41/46 | 5 | 6.0000 | 6.0000 | 0.0000 |
| wake-and-bake | 143/154 | 11 | 14.0000 | 14.0000 | 0.0000 |

**Total keyed 15069/16226.** Unallocated **1157 rows**, amount **28025828.6300** (burn, not mint).
