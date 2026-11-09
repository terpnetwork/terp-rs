# Zk-DAO

- register account to be able to have msgs authorized by one of dao members using zk-proofs

zk-inclusion proof + nullifier

<!-- *improve how we impelement this so that:

an account owned by a DAO where a zk-proof is required, prooving the following statements:

- "I know the secret key paired of an specific public key"
- "this public key is currently a member of the accounts DAO at this current height"
- "the action being performed has been authorized to do so"

## Requirements

- specific light client registerd contains remote membership root used in proof verification  (light client is how state membership transitions )
- utillize current vote-sdk key usage design for of key parigin and msg authorization 

### Circuit Proving Keys

### Proof

### Instances

Instances are the public inputs to circuits. The following are automatically provided by the blockchain application:
- block_height
- membership_merkle_root
- authorization_merkle_root -->

- curate zk-dao to be powered by inclusion proofs of dao membership, in design that is modular so that we can have support for various methods daos support memberships:
    - initial implementaitons are :
        - dao-voting-cw4 
        - cw721-staked
        - voting-token-staked
    
- expects blake3 hash of iavl merkle path for membership proofs (reference our latest wasm-terp design for blake3 membership proof)
- ideally uses registration of light client design (https://github.com/cosmos/ibc-go/tree/main/modules/light-clients/09-localhost) probably would be ideal usage design of this 