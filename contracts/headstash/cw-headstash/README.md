# Cw-Headstash Proof System

Singleton implementing proof system for consuming headstash notes.

## TODO

- implement as WAVS instance
- broadcast msgs via vote extensions
- query on-chain contract for merkle roots
- update on-chain contracts on state transitions
- implement privacy enhancement features

## BLS12-381 Support

### Signature Aggregation

 Multiple BLS signatures can be combined into a single signature of constant size. This is huge for scalability - whether you have 3 or 3000 signers, the final signature remains the same compact size (about 48 bytes).
Signature Aggregation: Multiple BLS signatures can be combined into a single signature of constant size. This is huge for scalability - whether you have 3 or 3000 signers, the final signature remains the same compact size (about 48 bytes).

**Accountability**: Unlike some multi-signature schemes, BLS aggregation can maintain accountability - you can prove which specific parties participated in creating an aggregated signature, which is crucial for governance and audit trails.

### Key Rotation

Key rotation is critical for long-term security, and BLS12-381 proof of ownership functions facilitate this in several ways:

**Secure Handover**: When rotating keys, you need to prove you legitimately own the old key before authorizing a new one. BLS signatures can create a cryptographic chain of custody - the old key signs a message authorizing the new public key, creating an auditable transition.

**Threshold Key Rotation**: In systems where keys are shared among multiple parties (using threshold cryptography), BLS12-381 enables coordinated key updates where a threshold of participants must prove ownership of their key shares to authorize rotation.

### Questions

- do i need to store all the keys of the set to assert the threshold requirements are met when messages are incoming?

- what needs to be provided when we are rotating just one key out of this list?

Rotation keys accepts a list of tuples containing the old key to rotate out with a new key. This also requires the consensus of at least the threshold set for the operator set. In order

### Resources

- <https://eth2book.info/latest/part2/building_blocks/signatures/>
- <https://medium.com/harmony-one/exploring-bls-keys-on-the-harmony-protocol-understanding-generation-management-and-use-cases-b8722f7219fc>

## TODO

- cw-json-filter support: filter preinput for tokens for prevention in duplicates

```
                                      ╭──────────────────────╮
                                      │  MsgAddAuthenticator │
                                      │   config arrives     │
                                      ╰─────────△────────────╯
                                                │
                                          ┌─────▼─────┐
                                          │ on_auth_  │
                                          │  added()  │
                                          │ validate  │
                                          │ + store   │
                                          └─────△─────┘
                                                │
                                                ▼
                              ┌───────────────────────────────────────┐
                              │        AUTHENTICATOR NOW ACTIVE       │
                              └─────────────────△───────────────────────△───────────┘
                                │                         │
                ┌───────────────▼───────┐   ┌─────────────▼──────────────┐
                │   Tx comes in         │   │   MsgRemoveAuthenticator   │
                │   msgs + signatures   │   └─────────────△──────────────┘
                └─────────────△─────────┘                 │
                              │                       ┌─────▼─────┐
            ┌─────────────────▼─────────────────┐     │ on_auth_  │
            │       on_auth_request()          │     │ removed() │
            │      stateless validation          │     │  cleanup  │
            │   (MUST NOT mutate state)          │     └─────△─────┘
            └─────────────────△─────────────────┘           │
                              │                             │
                  ┌───────────▼───────────┐                 │
                  │   AUTH PASSES         │                 │
                  └───────────△───────────┘                 │
                              │                             │
            ┌─────────────────▼─────────────────┐           │
            │          on_auth_track()          │           │
            │    safe to mutate state now      │           │
            │   (committed even if exec fails) │           │
            └─────────────────△─────────────────┘           │
                              │                             │
                              ▼                             │
                  ┌─────────────────────┐                   │
                  │   Handler Executes   │                   │
                  │   (normal msg logic) │                   │
                  └───────────△─────────┘                   │
                              │                             │
                  ┌───────────▼───────────┐                 │
                  │   on_auth_confirm()   │                 │
                  │ post-exec checks     │                 │
                  │ can REVERT whole tx  │                 │
                  └───────────△───────────┘                 │
                              │                             │
                  ┌───────────▼───────────┐                 │
                  │   TX COMMITTED        ◀─────────────────┘
                  │   (or reverted)   │
                  └───────────────────────┘

                          ║║║║║║║║║║║║║║║║║║║║║║║║║║║║║║
                          ║   RIVER OF AUTHENTICATION   ║
                          ║ process_sudo_auth() routes ║
                          ║   every arrow above        ║
                          ╚════════════════════════════╝
```