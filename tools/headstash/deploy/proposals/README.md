# Headstash proposals (drafts, not submitted)

- No store-code/instantiate proposal: mainnet `code_upload_access` and instantiate permission
  are Everybody (read 2026-10-26).
- `submit-proposal.template.json`: one gov proposal with `MsgStoreFullCircuit` (note circuit) and
  `MsgCommunityPoolSpend` to the contract and operators. Its `MsgRegisterChain` (headstash-claim oracle
  source) is dropped unless `LANE_PATH` is true; `<!--LANE-->`/`<!--DIRECT-->` blocks in the .md files
  are kept or removed the same way. Render with `../render.py`
  (output `../rendered/submit-proposal.json`), then
  `terpd tx gov submit-proposal <file> --from <proposer> --chain-id morocco-1`.
- `proposal-body.md`: full text for the metadata URI. `forum-post.md`: pre-proposal post.
- #32 funds: old contract balance is 0; nothing recoverable.
