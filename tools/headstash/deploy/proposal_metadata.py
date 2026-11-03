#!/usr/bin/env python3
"""Write the x/gov v1 ProposalMetadata JSON for the three Headstash 2026 proposals (A deploy, B1 circuit, B2 fund + bind).

Schema: cosmos-sdk x/gov/types/metadata.go ProposalMetadata
(title, authors, summary, details, proposal_forum_url, vote_option_context).
The chain stores only the URI (<= 255 B, MaxMetadataLen); these files are hosted off-chain.
Usage: proposal_metadata.py decisions.mainnet.json rendered-mainnet/sizes.json
Output: proposal-metadata/{01-deploy.json,02-circuit-store.json,03-fund-bind.json,SHA256SUMS}
"""
import hashlib, json, os, sys

HERE = os.path.dirname(os.path.abspath(__file__))
dec = json.load(open(sys.argv[1])); rep = json.load(open(sys.argv[2]))
forum = dec["FORUM_URL"]
if forum.startswith("{{"): sys.exit("FATAL FORUM_URL unset")
fe = rep["frontend"]
F = dict(contract=rep["contract_addr_predicted"], wasm=rep["wasm_sha256"], wasm_b=rep["wasm_bytes"],
         fe=fe["addr_predicted"], fe_wasm=fe["wasm_sha256"], fe_b=fe["wasm_bytes"], man=fe["manifest_sha256"],
         vk="34d11437d957554f4008377dece5fac0f047e0e9cc012d15b3d09448a6cc5398",
         note_root="c6ffae12fc3e5aa7f40a4f5b45e141b18362bdb06a798cc5c234ab5b5105a3b0",
         id_root="ffad92213f2c2001d73e56938064b0f2764535f220e16256abcd235f20338a57",
         gov="terp10d07y265gmmuvt4z0w9aw880jnsr700jag6fuq", src=dec.get("SOURCE_COMMIT", ""))
AUTHORS = ["Terp Network"]
COMMUNITIES = ("EVM: buddah-bears, cannabuddies, carta-beta-gang, chronic-token, crypto-canna-club, cryptowizards, "
               "galacktic-gang, heady-pipe-society, hippie-life-krew, monster-buds, rebud, secret-sesh, shurlok, special-k, "
               "wake-and-bake. Solana: stoned-ape-club.")
OVERVIEW = """## Headstash 2026 in short
A private TERP airdrop to holders from 15 EVM communities plus the Solana community stoned-ape-club:
17,791 eligible identities (15,744 EVM, 2,047 Solana) share 98,841 notes in 12 denominations (1 to 5,000 TERP),
totalling 66,456,669 TERP of the 67,000,000 TERP budget. 37 addresses were excluded (burn/zero addresses,
contracts, Safes and program-owned accounts).

Communities: %s

How a claim works:
1. Register: the holder signs a native registration message with the wallet that held the NFT
   (EIP-191 ECDSA for EVM, ed25519 for Solana). The message binds a note-key commitment to the eligible amount.
2. Claim: the holder proves ownership of a note with the Halo2 note circuit (vk_id 3, K=11) verified on-chain by
   the Terp circuit module, so the claiming Terp address is not linked to the source wallet on-chain.
3. Claim fees are covered by fee grants, funded by the foundation DAO (budget 4,761.7 THIOL), so claimers need no TERP or THIOL.

Governance and safety:
- x/gov (%s) is the wasm admin and the owner of both contracts. WAVS is disabled for this campaign.
- Claim cutoff at block 44,444,444.
- Emergency, before the cutoff: x/gov (and only x/gov, as admin and owner) can execute EmergencyReclaim, which sends the whole
  contract balance back to the community pool (MsgFundCommunityPool) and permanently halts claims and registration.
- Pause: x/gov, and the pauser set at deploy (the foundation DAO terp14w2qva6dx6wcsmq5fvplh7cr7nvptejznyvpe5hp5mtyqhxxjamsz3kw2w), can Pause/Resume claims and registration at once.
  The pauser can do nothing else (no funds, no config). Reclaim and the post-cutoff burn still work while paused; a pause does not extend the cutoff.
- After the cutoff reclaim is refused: unclaimed TERP can only be burned (SweepUnclaimed, BankMsg::Burn); it does not return to any wallet.
- The claim page is stored on-chain in a hash-verified frontend contract: every file is checked against a manifest hash published by this proposal.
- Post-claim privacy via Penumbra is planned; the current frontend only contains a preview stub for it.
"""

def details(extra):
    return (OVERVIEW % (COMMUNITIES, F["gov"])) + extra + ("\nSource commit: %s\n" % F["src"] if F["src"] else "")

HASHES = """
Hashes to check:
- cw_headstash.wasm sha256 %s (%d bytes), predicted address %s
- cw_headstash_frontend.wasm sha256 %s (%d bytes), predicted address %s
- page manifest v1 sha256 %s
- verifying key sha256(params||vk) %s
- note eligibility tree root (depth 17): %s
- identity tree root (depth 15): %s
""" % (F["wasm"], F["wasm_b"], F["contract"], F["fe_wasm"], F["fe_b"], F["fe"], F["man"], F["vk"], F["note_root"], F["id_root"])
VOTE = ("No: do not do it. No with veto: the proposal is spam or harmful. Abstain: no position.")
deploy = {
    "title": "Headstash 2026 (A): deploy cw-headstash and the claim page",
    "authors": AUTHORS,
    "summary": ("Step A of 3, funds nothing: instantiates cw-headstash (sha256 %s) at %s and the on-chain claim page host "
                "cw-headstash-frontend (sha256 %s) at %s, both with x/gov as admin and owner, and publishes claim page "
                "manifest v1 (sha256 %s). 1 uterp moves from the community pool (instantiate minimum). No circuit and no "
                "campaign: claims stay disabled until proposal B2." % (F["wasm"], F["contract"], F["fe_wasm"], F["fe"], F["man"])),
    "details": details("""
## What this proposal does
1. Community pool spend of 1 uterp to x/gov (cw-headstash refuses an instantiate without at least 1 token).
2. Instantiate2 of cw-headstash-frontend (admin and owner x/gov) and publish of page manifest v1; the page activates once every chunk is uploaded (permissionless, hash-checked).
3. Instantiate2 of cw-headstash (admin and owner x/gov, WAVS disabled) with that 1 uterp. Claims and registration are refused until B2 binds the circuit.

Nothing else is funded. Next: B1 stores the note circuit, B2 opens the campaign, binds it and funds the contract.
""" + HASHES),
    "proposal_forum_url": forum,
    "vote_option_context": "Yes: deploy both contracts with x/gov as admin and owner. " + VOTE,
}
circuit = {
    "title": "Headstash 2026 (B1): store the note circuit",
    "authors": AUTHORS,
    "summary": ("Step B1 of 3: stores the Headstash vk_id 3 Halo2 note circuit (K=11, verifying-key hash sha256(params||vk) %s) "
                "in the circuit module with x/gov as sender, for the cw-headstash contract deployed by proposal A at %s. "
                "Nothing is funded; proposal B2 binds this circuit." % (F["vk"], F["contract"])),
    "details": details("""
## What this proposal does
Stores the note circuit (vk_id 3, K=11) with x/gov as the sender. Nothing is funded by this proposal.
""" + HASHES),
    "proposal_forum_url": forum,
    "vote_option_context": "Yes: store the Headstash note circuit so proposal B2 can bind it. " + VOTE,
}
fund = {
    "title": "Headstash 2026 (B2): bind the note circuit and fund cw-headstash",
    "authors": AUTHORS,
    "summary": ("Step B2 of 3: opens the campaign on cw-headstash %s (deployed by proposal A, x/gov admin and owner), binds the "
                "note circuit from B1 and sends 66,456,668.999999 TERP from the community pool straight to the contract "
                "(66,456,669 TERP with the 1 uterp from A) for a private airdrop to 17,791 identities (98,841 notes) from 15 EVM "
                "communities and Solana stoned-ape-club. Cutoff block 44,444,444: before it x/gov can reclaim everything to the "
                "community pool in an emergency; after it unclaimed TERP is burned." % F["contract"]),
    "details": details("""
## What this proposal does
1. SetEpochCampaign on cw-headstash (cutoff 44,444,444, burn of unclaimed after the cutoff).
2. BindNoteCircuit with the circuit stored by B1 (the contract checks sha256 of the stored circuit against the verifying-key hash).
3. Community pool spend of 66,456,668,999,999 uterp straight to the contract.

The 9,632.607378 TERP leaf rounding remainder and the 533,698.39 TERP not allocated to notes are not sent: they stay in the community pool.
For comparison, Proposal #32 (December 2023) sent 43,476,089.03 TERP to code 8; that contract now holds 0.
""" + HASHES),
    "proposal_forum_url": forum,
    "vote_option_context": "Yes: fund and launch the Headstash 2026 private airdrop as described. " + VOTE,
}
out = os.path.join(HERE, "proposal-metadata"); os.makedirs(out, exist_ok=True)
sums = []
for name, m in (("01-deploy.json", deploy), ("02-circuit-store.json", circuit), ("03-fund-bind.json", fund)):
    assert set(m) == {"title", "authors", "summary", "details", "proposal_forum_url", "vote_option_context"}
    assert len(m["summary"].encode()) <= 40 * 255
    b = (json.dumps(m, indent=2, ensure_ascii=False) + "\n").encode()
    open(os.path.join(out, name), "wb").write(b)
    sums.append("%s  %s" % (hashlib.sha256(b).hexdigest(), name))
open(os.path.join(out, "SHA256SUMS"), "w").write("\n".join(sums) + "\n")
print("\n".join(sums))
