# terp-frost

Contract for the FROST **group verifying key**: rotation on execute, Schnorr
authorization on sudo.

- Instantiate stores `group_vk` (never empty) plus t-of-n seats.
- `ExecuteMsg::RotateFrostPubkey` / `AddRotateShare` always **Ok + write** (pending or fail event).
- t matching shares bound to `new_vk_commit` apply the new vk; commit/HKDF mismatch leaves the old vk.
- `sudo` `Authenticate` checks an osst secp256k1 Schnorr signature against the **applied** `group_vk` and does not write. A pending rotation does not change the authorizing key. `OnAuthRemoved` does not clear `group_vk`.
- Authorization `group_vk` is 66 lowercase hex characters of the 33-byte SEC1 point. The signature is 65 bytes: 33-byte SEC1 `R` || 32-byte scalar `z`. osst's 64-byte `Signature::to_bytes` drops y parity and is not accepted.
- The signed bytes are `sign_mode_tx_data.sign_mode_direct`.

`Authenticate` must not rotate. `x/smart-account` discards writes made from that call, so a rotate there would not land. Passkey/AnyOf selects the `MsgExecuteContract` for rotation. `msg_rotate_frost_pubkey` must not set `authenticator_type`.
