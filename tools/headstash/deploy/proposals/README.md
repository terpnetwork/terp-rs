# Headstash proposals (drafts, not submitted)

- No store-code/instantiate proposal: mainnet `code_upload_access` and instantiate permission
  are Everybody (read 2026-10-26).
- `submit-proposal.template.json`: one gov proposal with `MsgRegisterChain` (headstash-claim
  oracle source) and `MsgCommunityPoolSpend` to the contract. Render with `../render.py`
  (output `../rendered/submit-proposal.json`), then
  `terpd tx gov submit-proposal <file> --from <proposer> --chain-id morocco-1`.
- `proposal-body.md`: full text for the metadata URI. `forum-post.md`: pre-proposal post.
- #32 funds: old contract balance is 0; nothing recoverable.
