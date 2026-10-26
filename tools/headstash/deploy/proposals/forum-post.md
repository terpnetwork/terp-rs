# [Pre-proposal] Headstash 2026: private TERP airdrop to 16 partner communities

We are preparing a governance proposal to fund the Headstash airdrop from the community pool.

**Ask:** up to 67M TERP (exact leaf total {{ELIGIBILITY_TOTAL_UTERP}} uterp; addresses that cannot
sign (burn/zero, contracts, Safes, program-owned Solana accounts) are excluded and their share
stays in the pool; see eligibility/exclusions.csv) sent to a new cw-headstash contract whose
admin is the gov module, plus registering the `headstash-claim` oracle source.

**Why again?** Proposal #32 (2023) funded a public claim contract; it is empty now. This version
lets holders claim to any Terp address without linking it to their EVM/Solana wallet.

**How it works:** register once with your wallet signature, wait for the registration window to
close, then claim with a browser-generated proof. Unclaimed TERP is swept after the cutoff.

**Communities:** Buddah Bears, Cannabuddies, Carta Beta Gang, Chronic Token, Crypto Canna Club,
Crypto Wizards, Galacktic Gang, Heady Pipe Society, Hippie Life Krew, Monster Buds, Rebud,
Secret Sesh, Shurlok, Special K, Stoned Ape Club (Solana), Wake and Bake.

**Known trade-off:** payout amounts are public, so holders with a one-of-a-kind amount are
easier to link. We are discussing splitting allocations into fixed denominations; feedback welcome.

Questions on eligibility, timing (registration window, cutoff) and who should own the
contract are welcome below before we go on-chain.
