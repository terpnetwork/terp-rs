# terp-frost

Rotate contract for the ITL admin FROST **group verifying key**.

- Instantiate stores `group_vk` (never empty) plus t-of-n seats.
- `ExecuteMsg::RotateFrostPubkey` / `AddRotateShare` always **Ok + write** (pending or fail event).
- t matching shares bound to `new_vk_commit` apply the new vk; commit/HKDF mismatch leaves the old vk.
- **No `sudo` / Authenticate entry.** CosmwasmAuthenticatorV1 discards writes — forbidden for inbox and rotate.

Passkey/AnyOf selects the `MsgExecuteContract`. `msg_rotate_frost_pubkey` must not set `authenticator_type`.
