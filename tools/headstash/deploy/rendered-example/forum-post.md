# [Pre-proposal] Headstash 2026: private TERP airdrop to 16 partner communities

We are preparing a governance proposal to fund the Headstash airdrop from the community pool.

**Ask:** up to 67M TERP (exact leaf total 66466301607378 uterp; addresses that cannot
sign (burn/zero, contracts, Safes, program-owned Solana accounts) are excluded and their share
stays in the pool; see eligibility/exclusions.csv) sent to a new cw-headstash contract whose
admin is the gov module, which verifies claims itself and pays at once.
**Why again?** Proposal #32 (2023) funded a public claim contract; it is empty now. This version
lets holders claim to any Terp address without linking it to their EVM/Solana wallet.

**How it works:** register once with your wallet signature, wait for the registration window to
close, then claim with a browser-generated proof. Unclaimed TERP is swept after the cutoff.

**Communities:** Buddah Bears, Cannabuddies, Carta Beta Gang, Chronic Token, Crypto Canna Club,
Crypto Wizards, Galacktic Gang, Heady Pipe Society, Hippie Life Krew, Monster Buds, Rebud,
Secret Sesh, Shurlok, Special K, Stoned Ape Club (Solana), Wake and Bake.

| Community | Budget (TERP) | EVM wallets | Solana wallets | Excluded | Allocated (TERP) | Unallocated (TERP) |
|---|---:|---:|---:|---:|---:|---:|
| buddah-bears | 7,605,710 | 2,453 | 0 | 6 | 7,581,749.61 | 23,960.38 |
| cannabuddies | 2,369,078 | 238 | 0 | 1 | 2,350,175.78 | 18,902.21 |
| carta-beta-gang | 1,924,158 | 157 | 0 | 0 | 1,924,157.99 | 0.00 |
| chronic-token | 5,440,174 | 1,255 | 0 | 3 | 5,111,793.92 | 328,380.07 |
| crypto-canna-club | 10,192,106 | 4,405 | 0 | 7 | 10,178,856.26 | 13,249.73 |
| cryptowizards | 1,138,865 | 55 | 0 | 0 | 1,138,864.99 | 0.00 |
| galacktic-gang | 7,781,951 | 2,568 | 0 | 2 | 7,770,743.86 | 11,207.13 |
| heady-pipe-society | 736,470 | 23 | 0 | 0 | 736,470.00 | 0.00 |
| hippie-life-krew | 2,839,906 | 342 | 0 | 0 | 2,839,905.99 | 0.00 |
| monster-buds | 8,934,501 | 3,385 | 0 | 4 | 8,927,234.22 | 7,266.77 |
| rebud | 3,714,231 | 585 | 0 | 0 | 3,714,230.99 | 0.00 |
| secret-sesh | 3,525,302 | 527 | 0 | 2 | 3,417,438.28 | 107,863.71 |
| shurlok | 882,161 | 33 | 0 | 0 | 882,160.99 | 0.00 |
| special-k | 1,041,525 | 46 | 0 | 0 | 1,041,525.00 | 0.00 |
| stoned-ape-club | 6,968,176 | 0 | 2,059 | 12 | 6,945,307.65 | 22,868.34 |
| wake-and-bake | 1,905,686 | 154 | 0 | 0 | 1,905,685.99 | 0.00 |
| **Total** | **67,000,000** | **16,226** | **2,059** | **37** | **66,466,301.60** | **533,698.38** |

Allocated + unallocated + rounding dust (5,863 uterp) = 67,000,000 TERP exactly. Wallet counts are snapshot rows after exclusions; they merge into 17,791 identities (15,744 EVM, 2,047 Solana).

**Privacy:** allocations are split into 98841 fixed-denomination notes, so a claim only reveals
its denomination (each class has at least 5081 notes). Claiming right after registering, or
all notes at once, can still be correlated by time; the claim page adds random delays.
Questions on eligibility, timing (registration window, cutoff) and who should own the
contract are welcome below before we go on-chain.
