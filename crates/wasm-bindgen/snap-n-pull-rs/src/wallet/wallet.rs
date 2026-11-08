use std::num::NonZeroU32;

// use bip0039::{English, Mnemonic};
// use nonempty::NonEmpty;
// use secrecy::{ExposeSecret, SecretVec, Zeroize};
// use tonic::{
//     client::GrpcService,
//     codegen::{Body, Bytes, StdError},
// };

// use crate::BlockRange;
use crate::Error;
use crate::Network;
use group::ff::PrimeField;

// use pczt::roles::combiner::Combiner;
// use pczt::roles::prover::Prover;

use rand::Rng;
use rand::rngs::OsRng;
use serde::Deserialize;
// use pczt::roles::updater::Updater;
// use pczt::Pczt;
// use sapling::ProofGenerationKey;
// use serde::de::DeserializeOwned;
use serde::Serialize;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use zk_cosmwasm::TestPressSuite;
use zk_headstash::address::RecpAddr;
use zk_headstash::circuit::Instance;
use zk_headstash::keys::FullViewingKey;
use zk_headstash::keys::SpendingKey;
use zk_headstash::keys::{EligiblePk, EligibleSk, NullifierDerivingKey};
use zk_headstash::note::{Note, RandomSeed};
use zk_headstash::note::{NoteCommitment, Nullifier, Rho};

use zk_headstash::Anchor;
use zk_headstash::tree::MerkleHashOrchard;
use zk_headstash::tree::MerklePath;
use zk_headstash::value::HeadstashValue;

use crate::client::HeadstashClient;

/// Response from a headstash claim submission
#[derive(Debug, Clone, Serialize)]
pub struct ClaimResponse {
    /// Transaction hash
    pub tx_hash: String,
    /// Block height
    pub height: i64,
    /// Success/failure code
    pub code: u32,
    /// Transaction log
    pub raw_log: String,
}

/// Headstash metadata (from API, blockchain, or IPFS)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HeadstashInstance {
    pub ipfs_cid: String,
    pub ca: String,
    pub vk: Vec<u8>,
}

/// Headstash metadata (from API, blockchain, or IPFS)
#[derive(Debug, Clone)]
pub struct HeadstashMetadata {
    pub nf: Nullifier,
    /// mr: merkle-root
    pub mr: Vec<u8>,
    /// mp: merkle-path
    pub mp: Vec<Vec<u8>>,
    /// The value of the note
    pub hv: HeadstashValue,
    /// recp: recipient
    pub recp: Vec<u8>,
}

// #[derive(Debug, Clone)]
// pub struct NoteData {
//     /// nk: nullifier key derived from esk and rho
//     pub nk: NullifierDerivingKey,
//     /// The nullifier used to claim the note
//     pub nullifier: Nullifier,
//     /// The note commitment
//     pub commitment: NoteCommitment,
//     /// The value of the note
//     pub hv: HeadstashValue,
//     /// The fixed denomination index (leaf position in tree)
//     pub fdi: u64,
//     /// Whether this note has been spent
//     pub spent: bool,
// }

/// Proof data for smart account claim
#[derive(Debug, Clone, Serialize, Default)]
pub struct ProofData {
    /// The zkSNARK proof bytes
    pub proof: Vec<u8>,
    /// Public inputs for verification
    pub instances: Vec<Vec<u8>>,
    /// The nullifier being claimed
    pub nullifier: Vec<u8>,
}

// const BATCH_SIZE: u32 = 10000;

// /// constant that signals what's the minimum transparent balance for proposing a
// /// shielding transaction
// const SHIELDING_THRESHOLD: Zatoshis = Zatoshis::const_from_u64(100000);

/// # HeadstashWallet
///
/// A wallet is a manifold that is used to synchronized together with the blockchain & headstash-api.
/// It has the ability to store local records of spent note nullifiers & note-commitments, share & export these files via authenticated requests
///
/// ## Creating Note Nullifiers
///
/// TODO
///
/// ##  Syncing Headstash Notes
///
/// TODO
/// - determine which client to use (generic high performant grpc requests).meaning replace previous existing client struct`CompactTxStreamerClient` from zcash library with standard grpc client method
///
///
/// ## Building Transactions
///
/// TODO
///
///
///
/// HeadstashWallet - Database and API client for headstash operations
///
/// This is NOT a traditional wallet that stores secret keys.
/// It's a database that:
/// 1. Stores note data indexed by headstash ID
/// 2. Communicates with headstash API via gRPC
/// 3. Requests secret key from MetaMask only when needed
///
///
pub struct HeadstashWallet {
    /// Internal database for note data (nullifiers, commitments, etc.)
    // pub(crate) db: Arc<RwLock<W>>,
    /// Network configuration (mainnet/testnet)
    pub(crate) network: Network,
    /// gRPC client for headstash API communication
    /// (Similar to zcash's CompactTxStreamerClient but for headstash API)
    pub(crate) client: Option<HeadstashClient>,
}

impl HeadstashWallet {
    /// Create a new HeadstashWallet with database and network
    ///
    /// # Arguments
    /// * `db` - Database implementation for storing note data
    /// * `network` - Network configuration (mainnet/testnet)
    /// * `api_url` - Optional headstash API endpoint URL
    ///
    /// # Example
    /// ```rust,ignore
    /// let wallet = HeadstashWallet::new(
    ///     Network::MainNetwork,
    ///     Some("https://headstash-api.terp.network")
    /// ).await?;
    /// ```
    pub async fn new(network: Network, api_url: Option<&str>) -> Result<Self, Error> {
        let client = HeadstashClient::new(None, api_url).await?;

        Ok(Self {
            network,
            client: Some(client),
        })
    }

    /// Generate proof witness for headstash claim
    ///
    /// This generates the zkSNARK proof using the headstash circuit.
    ///
    /// # Arguments
    /// * `esk` - Secret key (transient)
    /// * `nullifier` - The nullifier being claimed
    /// * `metadata` - Headstash metadata (merkle root, circuit params, etc.)
    async fn gen_proof_witness(
        &self,
        esk: EligibleSk,
        hi: &HeadstashInstance,
        md: &HeadstashMetadata,
    ) -> Result<ProofData, Error> {
        Ok(ProofData {
            proof: TestPressSuite::new()
                .create_headstash_proof(
                    Anchor::from_bytes(md.mr.as_slice().try_into().expect("darg")).expect("bvad"),
                    MerklePath::from_parts(
                        1,
                        md.mp
                            .iter()
                            .map(|l| {
                                MerkleHashOrchard::from_bytes(l.as_slice().try_into().unwrap())
                                    .expect("darng")
                            })
                            .collect::<Vec<_>>()
                            .try_into()
                            .expect("Expected exactly 32 merkle path elements"),
                    ),
                    esk,
                    RecpAddr::new(md.recp.as_slice().try_into().expect("darn")),
                    md.hv,
                )
                .await
                .map_err(|e| Error::KeyDecoding(e.to_string()))?
                .as_ref()
                .to_vec(),
            instances: vec![],
            nullifier: md.nf.to_bytes().to_vec(),
        })
    }

    /// connects to headstash api client
    pub async fn client(&self) -> HeadstashClient {
        let client = HeadstashClient::new(None, Some("https://headstash-api.terp.network"))
            .await
            .unwrap();

        client
    }

    /// ## `generate_note_data`
    /// ### generates nullifer-key `nk` ,nullifier `nf`, and note-commitment `cm`
    pub fn generate_note_data(
        &self,
        esk: EligibleSk,
        rho: Rho,
        hv: HeadstashValue,
        recp: RecpAddr,
        rseed: [u8; 32],
    ) -> Result<(NullifierDerivingKey, Nullifier, NoteCommitment), Error> {
        let nk = NullifierDerivingKey::derive_from(esk, rho);
        let rseed = RandomSeed::from_bytes(rseed, &rho).expect("rseed input");

        // let fvk = FullViewingKey::from(
        //     &SpendingKey::from_bytes(esk.derive_pallas().to_repr()).expect("esk spending key"),
        // );

        let note = Note::from_parts(hv, recp, esk, rho, rseed)
            .into_option()
            .ok_or_else(|| Error::KeyDecoding("Failed to create valid note".into()))?;

        // Derive nullifier and commitment
        let nullifier = note.nullifier();
        let commitment = note.commitment();

        Ok((nk, nullifier, commitment))
    }

    // /// Store a new note in the database
    // ///
    // /// # Arguments
    // /// * `headstash_id` - The headstash contract address
    // /// * `esk` - Secret key from MetaMask (for generating nullifier/commitment)
    // /// * `rho` - Randomness value
    // /// * `fdi` - Leaf index
    // /// * `v` - Note value
    // pub async fn gen_claim(
    //     &self,
    //     headstash_id: String,
    //     esk: EligibleSk,
    //     rho: Rho,
    //     fdi: u64,
    //     recp: &[u8],
    //     hv: HeadstashValue,
    //     rseed: [u8; 32],
    // ) -> Result<(), Error> {
    //     let (nk, nullifier, commitment) =
    //         self.generate_note_data(esk, rho, fdi, recp, hv, rseed)?;

    //     let note_data = NoteData {
    //         nk,
    //         nullifier,
    //         commitment,
    //         hv,
    //         fdi,
    //         spent: false,
    //     };

    //     let mut db = self.db.write().await;
    //     db.gen_claim(&headstash_id, note_data)?;

    //     Ok(())
    // }

    // /// List all unspent notes for a headstash
    // ///
    // /// # Arguments
    // /// * `headstash_id` - The headstash contract address
    // pub async fn list_unspent_notes(&self, headstash_id: &String) -> Result<Vec<NoteData>, Error> {
    //     let db = self.db.read().await;
    //     db.list_unspent_notes(headstash_id)
    // }

    // /// List all spent notes for a headstash
    // ///
    // /// # Arguments
    // /// * `headstash_id` - The headstash contract address
    // pub async fn list_spent_notes(&self, headstash_id: &String) -> Result<Vec<NoteData>, Error> {
    //     let db = self.db.read().await;
    //     db.list_spent_notes(headstash_id)
    // }

    /// Claim a headstash allocation via manual contract call
    ///
    /// This generates the proof and submits directly to chain, expecting a headstash to allow nullifiers to be appended to the state.
    ///
    /// # Arguments
    /// * `headstash_id` - The headstash contract address
    /// * `esk` - Secret key from MetaMask (for signing)
    /// * `nullifier` - The nullifier of the note to claim
    ///
    /// # Workflow
    /// 1. Get note data from database
    /// 2. Generate proof (via HeadstashSuite)
    /// 3. Form CosmosSDK message
    /// 4. Broadcast to chain
    /// 5. Mark note as spent
    // pub async fn claim_headstash_via_manually(
    //     &self,
    //     headstash_id: &str,
    //     esk: &EligibleSk,
    //     nullifier: &Nullifier,
    // ) -> Result<(), Error> {
    //     // 1. Generate note-data
    //     // let db = self.db.read().await;
    //     // let note_data = db
    //     //     .get_note_by_nullifier(&headstash_id, &nullifier)?
    //     //     .ok_or_else(|| Error::KeyDecoding("Note not found".into()))?;
    //     // drop(db);

    //     // 2. Generate proof
    //     // TODO: Implement proof generation via HeadstashSuite
    //     // let proof = generate_proof(esk, note_data)?;

    //     // 3. Form message
    //     // TODO: Create CosmosSDK message with proof and nullifier

    //     // 4. Broadcast
    //     // TODO: Submit transaction to chain

    //     // 5. Mark as spent
    //     // let mut db = self.db.write().await;
    //     // db.mark_note_spent(&headstash_id, &nullifier)?;

    //     Ok(())
    // }

    /// Claim via feegrant
    ///
    /// Requests feegrant from headstash-server before claiming.
    // pub async fn claim_headstash_via_feegrant(
    //     &self,
    //     grantee_addr: &String,
    //     headstash_id: &String,
    //     esk: &EligibleSk,
    //     nullifier: &Nullifier,
    // ) -> Result<(), Error> {
    //     // 1. Request feegrant from headstash-server
    //     // TODO: Implement feegrant request via gRPC client
    //     let c = self.client().await;
    //     let res = c
    //         .request_feegrant(headstash_id, grantee_addr, &nullifier.to_bytes())
    //         .await?;
    //     // 2. Wait for feegrant confirmation
    //     // TODO: Poll for feegrant status

    //     // 3. Proceed with claim (msg wrapped with feegrant)
    //     Ok(())
    // }

    /// Claim via smart account
    ///
    /// Uses smart account authenticator for gasless transactions.
    /// This is the primary claim method for headstash allocations.
    ///
    /// # Workflow
    /// 1. Check for cached verification key for this headstash
    /// 2. If not cached, fetch metadata (API → Blockchain → IPFS)
    /// 3. Generate proof witness using local circuit
    /// 4. Submit claim to headstash-api with smart account auth
    ///
    /// # Arguments
    /// * `hid` - Contract address
    /// * `esk` - Secret key from MetaMask (transient, not stored)
    /// * `nullifier` - The nullifier to claim
    pub async fn claim_headstash_via_smart_account(
        &self,
        hid: String,
        esk: EligibleSk,
        hmd: HeadstashMetadata,
    ) -> Result<ClaimResponse, Error> {
        let c = self
            .client
            .as_ref()
            .ok_or_else(|| Error::Js("No client configured".into()))?;

        // 1. Check for cached verification key
        let m = match !self.check_vk_cache(&hid).await? {
            true => {
                let m = c.get_headstash_instance(&hid).await?;
                self.cache_vk(&hid, &m.vk).await?;
                m
            }
            false => c.get_headstash_instance(&hid).await?,
        };

        // generate proof witness data
        let pd = self.gen_proof_witness(esk, &m, &hmd).await?;
        let r = c.submit_smart_account_claim(&hid, pd).await?;

        // 6. Mark as spent in local DB (when DB is implemented)
        // db.mark_note_spent(&headstash_id, &nullifier)?;

        Ok(r)
    }

    /// Check if verification key is cached for a headstash
    ///
    /// This minimizes bandwidth by avoiding re-downloading circuit keys.
    async fn check_vk_cache(&self, headstash_id: &str) -> Result<bool, Error> {
        // TODO: Check MetaMask snap storage for cached VK
        // For now, return false to always fetch
        Ok(false)
    }

    /// Cache verification key in local storage
    async fn cache_vk(&self, headstash_id: &str, vk: &[u8]) -> Result<(), Error> {
        // TODO: Store VK in MetaMask snap storage
        // This would use snap_manageState to persist the VK
        Ok(())
    }

    // /// Export encrypted spent note details
    // ///
    // /// Encrypts spent notes nullifier-key `nk`, the ranomness bytes, and the bytes to use across library.
    // /// This is used for syncing nullifier state across devices.
    // ///
    // /// # Arguments
    // /// * `headstash_id` - Contract address to export notes for
    // /// * `recipient_pk` - Public key to encrypt to
    // /// * `sender_sk` - Secret key for signing (proves origin)
    // pub async fn export_headstash_instance_yaml(
    //     &self,
    //     headstash_id: &String,
    //     recipient_pk: &EligiblePk,
    //     sender_sk: &EligibleSk,
    // ) -> Result<Vec<u8>, Error> {
    //     todo!()
    //     // let db = self.db.read().await;
    //     // let spent_notes = db.list_spent_notes(headstash_id)?;

    //     // // Convert to serializable format
    //     // let serialized_notes: Vec<SerializedNoteData> =
    //     //     spent_notes.iter().map(|n| n.into()).collect();

    //     // // Create nullifier state
    //     // let state = NullifierState {
    //     //     headstash_id: headstash_id.clone(),
    //     //     spent_notes: serialized_notes,
    //     // };

    //     // // Encrypt
    //     // let encrypted = encrypt_nullifier_state(&state, recipient_pk, sender_sk)?;

    //     // // Serialize encrypted payload
    //     // serde_json::to_vec(&encrypted)
    //     //     .map_err(|e| Error::Js(format!("Failed to serialize encrypted state: {}", e).into()))
    // }

    // /// Sync encrypted spent note details from endpoint
    // ///
    // /// Retrieves and decrypts spent notes, merging into local storage.
    // /// This allows synchronizing nullifier state across devices.
    // ///
    // /// # Arguments
    // /// * `headstash_id` - Contract address
    // /// * `recipient_sk` - Secret key to decrypt with
    // /// * `sender_pk` - Expected sender's public key (for verification)
    // /// * `encrypted_data` - Encrypted nullifier state from API
    // pub async fn sync_spent_note_details_encrypted(
    //     &self,
    //     headstash_id: &String,
    //     recipient_sk: &EligibleSk,
    //     sender_pk: &EligiblePk,
    //     encrypted_data: Vec<u8>,
    // ) -> Result<(), Error> {
    //     // 1. Deserialize encrypted payload
    //     let encrypted: EncryptedNullifierState =
    //         serde_json::from_slice(&encrypted_data).map_err(|e| {
    //             Error::Js(format!("Failed to deserialize encrypted data: {}", e).into())
    //         })?;

    //     // 2. Decrypt and verify
    //     let state = decrypt_nullifier_state(&encrypted, recipient_sk, sender_pk)?;

    //     // 3. Verify headstash ID matches
    //     if &state.headstash_id != headstash_id {
    //         return Err(Error::Js("Headstash ID mismatch".into()));
    //     }

    //     // 4. Merge notes into storage (only add new ones, don't overwrite)
    //     let mut db = self.db.write().await;
    //     for serialized_note in state.spent_notes {
    //         // Convert back to NoteData
    //         // For now, we'll skip notes we don't already have since we need
    //         // the full note data structures, not just serialized bytes
    //         // In a real implementation, you'd reconstruct the full structures

    //         // This is a simplified merge - in production, you'd want more sophisticated logic
    //         let nullifier = Nullifier::from_bytes(
    //             serialized_note
    //                 .nullifier
    //                 .as_slice()
    //                 .try_into()
    //                 .map_err(|_| Error::Js("Invalid nullifier bytes".into()))?,
    //         )
    //         .expect("nullifier derivation");

    //         // // Check if we already have this note
    //         // if let Ok(Some(_)) = db.get_note_by_nullifier(headstash_id, &nullifier) {
    //         //     // Mark it as spent if it isn't already
    //         //     db.mark_note_spent(headstash_id, &nullifier)?;
    //         // }
    //     }

    //     Ok(())
    // }

    // /// Upload nullifier state to headstash-api for sync
    // ///
    // /// This encrypts and uploads spent nullifier state to the API server
    // /// for cross-device synchronization.
    // ///
    // /// # Arguments
    // /// * `headstash_id` - Contract address
    // /// * `recipient_pk` - Public key (usually your own) to encrypt to
    // /// * `sender_sk` - Secret key for signing
    // pub async fn upload_nullifier_state(
    //     &self,
    //     headstash_id: &String,
    //     recipient_pk: &EligiblePk,
    //     sender_sk: &EligibleSk,
    // ) -> Result<(), Error> {
    //     // Export encrypted state
    //     let encrypted_data = self
    //         .export_spent_note_details_encrypted(headstash_id, recipient_pk, sender_sk)
    //         .await?;

    //     // Sign the encrypted data
    //     use sha3::{Digest, Sha3_256};
    //     let data_hash = Sha3_256::digest(&encrypted_data);

    //     // Create signature message
    //     let mut sig_message = Vec::new();
    //     sig_message.extend_from_slice(headstash_id.as_bytes());
    //     sig_message.extend_from_slice(&data_hash);

    //     // Sign
    //     let signature = sign_message(&sig_message, sender_sk)?;

    //     // Upload via client
    //     let client = self
    //         .client
    //         .as_ref()
    //         .ok_or_else(|| Error::Js("No client configured".into()))?;

    //     client
    //         .upload_nullifier_state(headstash_id, encrypted_data, signature)
    //         .await?;

    //     Ok(())
    // }

    // /// Download and sync nullifier state from headstash-api
    // ///
    // /// This downloads encrypted nullifier state from the API server
    // /// and merges it into local storage.
    // ///
    // /// # Arguments
    // /// * `headstash_id` - Contract address
    // /// * `recipient_sk` - Secret key to decrypt with
    // /// * `sender_pk` - Expected sender's public key (for verification)
    // pub async fn download_and_sync_nullifier_state(
    //     &self,
    //     headstash_id: &String,
    //     recipient_sk: &EligibleSk,
    //     sender_pk: &EligiblePk,
    // ) -> Result<(), Error> {
    //     // Download from API
    //     let client = self
    //         .client
    //         .as_ref()
    //         .ok_or_else(|| Error::Js("No client configured".into()))?;

    //     let recipient_pk_bytes = recipient_sk.epk().0.serialize();
    //     let encrypted_data = client
    //         .download_nullifier_state(headstash_id, &recipient_pk_bytes)
    //         .await?;

    //     // Sync into storage
    //     self.sync_spent_note_details_encrypted(
    //         headstash_id,
    //         recipient_sk,
    //         sender_pk,
    //         encrypted_data,
    //     )
    //     .await?;

    //     Ok(())
    // }
}

/// In-memory implementation of HeadstashApiDbInstance (for testing and development)
#[derive(Debug, Clone, Default)]
pub struct MemoryHeadstashDb {
    // Maps: String → (Nullifier bytes → NoteData)
    storage: HashMap<String, HashMap<[u8; 32], ()>>,
}

// ============================================================================
// Helper Functions
// ============================================================================

/// Sign a message with secp256k1
fn sign_message(message: &[u8], sk: &EligibleSk) -> Result<Vec<u8>, Error> {
    use secp256k1::{Message, Secp256k1};
    use sha3::{Digest, Sha3_256};

    let secp = Secp256k1::new();

    // Hash the message
    let msg_hash = Sha3_256::digest(message);
    let message = Message::from_digest_slice(&msg_hash)
        .map_err(|e| Error::Js(format!("Invalid message: {}", e).into()))?;

    // Sign
    let signature = secp.sign_ecdsa(message, &sk.secret_key());

    Ok(signature.serialize_compact().to_vec())
}

pub struct Wallet<W> {
    /// Internal database used to maintain wallet data (e.g. accounts, transactions, cached blocks)
    pub(crate) db: Arc<RwLock<W>>,
    // // gRPC client used to connect to a lightwalletd instance for network data
    // pub(crate) client: CompactTxStreamerClient<T>,
    pub(crate) network: Network,
    pub(crate) min_confirmations: NonZeroU32,
    /// Note management: the number of notes to maintain in the wallet
    pub(crate) target_note_count: usize,
    /// Note management: the minimum allowed value for split change amounts
    pub(crate) min_split_output_value: u64,
}

impl<W: Clone> Clone for Wallet<W> {
    fn clone(&self) -> Self {
        Self {
            db: self.db.clone(),
            // client: self.client.clone(),
            network: self.network,
            min_confirmations: self.min_confirmations,
            target_note_count: self.target_note_count,
            min_split_output_value: self.min_split_output_value,
        }
    }
}

// impl<W, T, AccountId, NoteRef> Wallet<W, T>
// where
//     W: WalletRead<AccountId = AccountId>
//         + WalletWrite
//         + InputSource<
//             AccountId = <W as WalletRead>::AccountId,
//             Error = <W as WalletRead>::Error,
//             NoteRef = NoteRef,
//         > + WalletCommitmentTrees,

//     AccountId: Copy
//         + Debug
//         + Eq
//         + Hash
//         + Default
//         + Send
//         + ConditionallySelectable
//         + Serialize
//         + DeserializeOwned
//         + 'static,
//     NoteRef: Copy + Eq + Ord + Debug,
//     Error: From<<W as WalletRead>::Error>,

//     <W as WalletRead>::Error: std::error::Error + Send + Sync + 'static,
//     <W as WalletCommitmentTrees>::Error: std::error::Error + Send + Sync + 'static,

//     // GRPC connection Trait Bounds
//     T: GrpcService<tonic::body::Body> + Clone,
//     T::Error: Into<StdError>,
//     T::ResponseBody: Body<Data = Bytes> + std::marker::Send + 'static,
//     <T::ResponseBody as Body>::Error: Into<StdError> + std::marker::Send,
// {
//     /// Create a new instance of a Zcash wallet for a given network
//     pub fn new(
//         db: W,
//         client: T,
//         network: Network,
//         min_confirmations: NonZeroU32,
//     ) -> Result<Self, Error> {
//         Ok(Wallet {
//             db: Arc::new(RwLock::new(db)),
//             client: CompactTxStreamerClient::new(client),
//             network,
//             min_confirmations,
//             target_note_count: 4,
//             min_split_output_value: 10000000,
//         })
//     }

//     /// Add a new account to the wallet
//     ///
//     /// # Arguments
//     /// seed_phrase - mnemonic phrase to initialise the wallet
//     /// account_id - The HD derivation index to use. Can be any integer
//     /// birthday_height - The block height at which the account was created, optionally None and the current height is used
//     ///
//     pub async fn create_account(
//         &self,
//         account_name: &str,
//         seed_phrase: &str,
//         account_hd_index: u32,
//         birthday_height: Option<u32>,
//         key_source: Option<&str>,
//     ) -> Result<AccountId, Error> {
//         // decode the mnemonic and derive the first account
//         let (usk, seed_fp) = usk_from_seed_str(seed_phrase, account_hd_index, &self.network)?;
//         let ufvk = usk.to_unified_full_viewing_key();

//         let derivation =
//             Zip32Derivation::new(seed_fp, zip32::AccountId::try_from(account_hd_index)?);

//         tracing::info!("Key successfully decoded. Importing into wallet");

//         self.import_account_ufvk(
//             account_name,
//             &ufvk,
//             birthday_height,
//             AccountPurpose::Spending {
//                 derivation: Some(derivation),
//             },
//             key_source,
//         )
//         .await
//     }

//     pub async fn import_ufvk(
//         &self,
//         account_name: &str,
//         ufvk: &UnifiedFullViewingKey,
//         purpose: AccountPurpose,
//         birthday_height: Option<u32>,
//         key_source: Option<&str>,
//     ) -> Result<AccountId, Error> {
//         self.import_account_ufvk(account_name, ufvk, birthday_height, purpose, key_source)
//             .await
//     }

//     /// Helper method for importing an account directly from a Ufvk or from seed.
//     async fn import_account_ufvk(
//         &self,
//         account_name: &str,
//         ufvk: &UnifiedFullViewingKey,
//         birthday_height: Option<u32>,
//         purpose: AccountPurpose,
//         key_source: Option<&str>,
//     ) -> Result<AccountId, Error> {
//         tracing::info!("Importing account with Ufvk: {:?}", ufvk);
//         let mut client = self.client.clone();
//         let birthday = match birthday_height {
//             Some(height) => height,
//             None => {
//                 let chain_tip: u32 = client
//                     .get_latest_block(service::ChainSpec::default())
//                     .await?
//                     .into_inner()
//                     .height
//                     .try_into()
//                     .expect("block heights must fit into u32");
//                 chain_tip - 100
//             }
//         };
//         // Construct an `AccountBirthday` for the account's birthday.
//         let birthday = {
//             // Fetch the tree state corresponding to the last block prior to the wallet's
//             // birthday height. NOTE: THIS APPROACH LEAKS THE BIRTHDAY TO THE SERVER!
//             let request = service::BlockId {
//                 height: (birthday - 1).into(),
//                 ..Default::default()
//             };
//             let treestate = client.get_tree_state(request).await?.into_inner();
//             AccountBirthday::from_treestate(treestate, None).map_err(|_| Error::Birthday)?
//         };

//         Ok(self
//             .db
//             .write()
//             .await
//             .import_account_ufvk(account_name, ufvk, &birthday, purpose, key_source)?
//             .id())
//     }

//     pub async fn suggest_scan_ranges(&self) -> Result<Vec<BlockRange>, Error> {
//         Ok(self.db.read().await.suggest_scan_ranges().map(|ranges| {
//             ranges
//                 .iter()
//                 .map(|scan_range| {
//                     BlockRange(
//                         scan_range.block_range().start.into(),
//                         scan_range.block_range().end.into(),
//                     )
//                 })
//                 .collect()
//         })?)
//     }

//     ///
//     /// Create a transaction proposal to send funds from the wallet to a given address
//     ///
//     pub async fn propose_transfer(
//         &self,
//         account_id: AccountId,
//         to_address: ZcashAddress,
//         value: u64,
//     ) -> Result<Proposal<StandardFeeRule, NoteRef>, Error> {
//         let input_selector = GreedyInputSelector::new();

//         let change_strategy = MultiOutputChangeStrategy::new(
//             StandardFeeRule::Zip317,
//             None,
//             ShieldedProtocol::Orchard,
//             DustOutputPolicy::default(),
//             SplitPolicy::with_min_output_value(
//                 NonZeroUsize::new(self.target_note_count)
//                     .ok_or(Error::FailedToCreateTransaction)?,
//                 Zatoshis::from_u64(self.min_split_output_value)?,
//             ),
//         );
//         let request = TransactionRequest::new(vec![Payment::without_memo(
//             to_address,
//             Zatoshis::from_u64(value)?,
//         )])?;

//         tracing::info!("Chain height: {:?}", self.db.read().await.chain_height()?);
//         tracing::info!(
//             "target and anchor heights: {:?}",
//             self.db
//                 .read()
//                 .await
//                 .get_target_and_anchor_heights(self.min_confirmations)?
//         );
//         let mut db = self.db.write().await;
//         let proposal = propose_transfer::<_, _, _,_, <W as WalletCommitmentTrees>::Error>(
//             &mut *db,
//             &self.network,
//             account_id,
//             &input_selector,
//             &change_strategy,
//             request,
//             self.min_confirmations,
//         )
//         .map_err(|_e| Error::Generic("something bad happened when calling propose transfer. Possibly insufficient balance..".to_string()))?;
//         tracing::info!("Proposal: {:#?}", proposal);
//         Ok(proposal)
//     }

//     ///
//     /// Do the proving and signing required to create one or more transaction from the proposal. Created transactions are stored in the wallet database.
//     ///
//     /// Note: At the moment this requires a USK but ideally we want to be able to hand the signing off to a separate service
//     ///     e.g. browser plugin, hardware wallet, etc. Will need to look into refactoring librustzcash create_proposed_transactions to allow for this
//     ///
//     pub async fn create_proposed_transactions(
//         &self,
//         proposal: Proposal<StandardFeeRule, NoteRef>,
//         usk: &UnifiedSpendingKey,
//     ) -> Result<NonEmpty<TxId>, Error> {
//         let prover = LocalTxProver::bundled();
//         let mut db = self.db.write().await;
//         let transactions = create_proposed_transactions::<
//             _,
//             _,
//             <MemoryWalletDb<Network> as InputSource>::Error,
//             _,
//             <StandardFeeRule as FeeRule>::Error,
//             _,
//         >(
//             &mut *db,
//             &self.network,
//             &prover,
//             &prover,
//             usk,
//             OvkPolicy::Sender,
//             &proposal,
//         )
//         .map_err(|_| Error::FailedToCreateTransaction)?;
//         Ok(transactions)
//     }

//     pub async fn send_authorized_transactions(&self, txids: &NonEmpty<TxId>) -> Result<(), Error> {
//         let mut client = self.client.clone();
//         for txid in txids.iter() {
//             let (txid, raw_tx) = self
//                 .db
//                 .read()
//                 .await
//                 .get_transaction(*txid)?
//                 .map(|tx| {
//                     let mut raw_tx = service::RawTransaction::default();
//                     tx.write(&mut raw_tx.data).unwrap(); // safe to unwrap here as we know the tx is valid
//                     (tx.txid(), raw_tx)
//                 })
//                 .ok_or(Error::TransactionNotFound(*txid))?;

//             let response = client.send_transaction(raw_tx).await?.into_inner();

//             if response.error_code != 0 {
//                 return Err(Error::SendFailed {
//                     code: response.error_code,
//                     reason: response.error_message,
//                 });
//             } else {
//                 tracing::info!("Transaction {} send successfully :)", txid);
//             }
//         }
//         Ok(())
//     }

//     ///
//     /// A helper function that creates a proposal, creates a transaction from the proposal and then submits it
//     ///
//     pub async fn transfer(
//         &self,
//         seed_phrase: &str,
//         account_hd_index: u32,
//         from_account_id: AccountId,
//         to_address: ZcashAddress,
//         value: u64,
//     ) -> Result<(), Error> {
//         let (usk, _) = usk_from_seed_str(seed_phrase, account_hd_index, &self.network)?;
//         let proposal = self
//             .propose_transfer(from_account_id, to_address, value)
//             .await?;
//         // TODO: Add callback for approving the transaction here
//         let txids = self.create_proposed_transactions(proposal, &usk).await?;

//         // send the transactions to the network!!
//         tracing::info!("Sending transactions");
//         self.send_authorized_transactions(&txids).await
//     }

//     pub async fn pczt_shield(&self, account_id: AccountId) -> Result<Pczt, Error> {
//         let change_strategy = MultiOutputChangeStrategy::new(
//             StandardFeeRule::Zip317,
//             None,
//             ShieldedProtocol::Orchard,
//             DustOutputPolicy::default(),
//             SplitPolicy::with_min_output_value(
//                 NonZeroUsize::new(self.target_note_count)
//                     .ok_or(Error::FailedToCreateTransaction)?,
//                 Zatoshis::from_u64(self.min_split_output_value)?,
//             ),
//         );

//         let input_selector = GreedyInputSelector::new();
//         let mut db = self.db.write().await;

//         // Shield all funds immediately
//         let max_height = match db.chain_height()? {
//             Some(max_height) => max_height,
//             // If we haven't scanned anything, there's nothing to do.
//             None => {
//                 return Err(Error::Generic(
//                     "Havent scanned yet, cant shield".to_string(),
//                 ))
//             }
//         };

//         let transparent_balances = db.get_transparent_balances(account_id, max_height)?;
//         let from_addrs = transparent_balances.into_keys().collect::<Vec<_>>();

//         let proposal = propose_shielding::<_, _, _, _, <W as WalletCommitmentTrees>::Error>(
//             &mut *db,
//             &self.network,
//             &input_selector,
//             &change_strategy,
//             SHIELDING_THRESHOLD, // use a shielding threshold above a marginal fee transaction plus some value like Zashi does.
//             &from_addrs,
//             account_id,
//             1, // librustzcash operates under the assumption of zero or one conf being the same but that could change.
//         )
//         .map_err(|e| Error::Generic(format!("Error when shielding: {:?}", e)))?;

//         let pczt = create_pczt_from_proposal::<
//             _,
//             _,
//             <MemoryWalletDb<Network> as InputSource>::Error,
//             _,
//             <StandardFeeRule as FeeRule>::Error,
//             _,
//         >(
//             &mut *db,
//             &self.network,
//             account_id,
//             OvkPolicy::Sender,
//             &proposal,
//         )
//         .map_err(|_| Error::PcztCreate)?;

//         Ok(pczt)
//     }
//     ///
//     /// Create a PCZT
//     ///
//     pub async fn pczt_create(
//         &self,
//         account_id: AccountId,
//         to_address: ZcashAddress,
//         value: u64,
//     ) -> Result<Pczt, Error> {
//         // Create the PCZT.
//         let change_strategy = MultiOutputChangeStrategy::new(
//             StandardFeeRule::Zip317,
//             None,
//             ShieldedProtocol::Orchard,
//             DustOutputPolicy::default(),
//             SplitPolicy::with_min_output_value(
//                 NonZeroUsize::new(self.target_note_count)
//                     .ok_or(Error::FailedToCreateTransaction)?,
//                 Zatoshis::from_u64(self.min_split_output_value)?,
//             ),
//         );

//         let input_selector = GreedyInputSelector::new();
//         let request = TransactionRequest::new(vec![Payment::without_memo(
//             to_address,
//             Zatoshis::from_u64(value)?,
//         )])?;
//         let mut db = self.db.write().await;
//         let proposal = propose_transfer::<_, _, _,_, <W as WalletCommitmentTrees>::Error>(
//             &mut *db,
//             &self.network,
//             account_id,
//             &input_selector,
//             &change_strategy,
//             request,
//             self.min_confirmations,
//         )
//             .map_err(|e| Error::Generic(format!("something bad happened when calling propose transfer. Possibly insufficient balance... {:?}", e)))?;
//         tracing::info!("Proposal: {:#?}", proposal);
//         let pczt = create_pczt_from_proposal::<
//             _,
//             _,
//             <MemoryWalletDb<Network> as InputSource>::Error,
//             _,
//             <StandardFeeRule as FeeRule>::Error,
//             _,
//         >(
//             &mut *db,
//             &self.network,
//             account_id,
//             OvkPolicy::Sender,
//             &proposal,
//         )
//         .map_err(|_| Error::PcztCreate)?;
//         Ok(pczt)
//     }

//     ///
//     /// Prove a PCZT
//     ///
//     pub async fn pczt_prove(
//         &self,
//         pczt: Pczt,
//         sapling_proof_gen_key: Option<ProofGenerationKey>,
//     ) -> Result<Pczt, Error> {
//         // Add Sapling proof generation key.
//         // TODO: Check to see if there is any sapling in here in the first place
//         let pczt = if let Some(sapling_proof_gen_key) = sapling_proof_gen_key {
//             Updater::new(pczt)
//                 .update_sapling_with(|mut updater| {
//                     let non_dummy_spends = updater
//                         .bundle()
//                         .spends()
//                         .iter()
//                         .enumerate()
//                         // Dummy spends will already have a proof generation key.
//                         .filter(|(_, spend)| spend.proof_generation_key().is_none())
//                         .map(|(index, spend)| {
//                             (
//                                 index,
//                                 spend
//                                     .zip32_derivation()
//                                     .as_ref()
//                                     .map(|d| (*d.seed_fingerprint(), d.derivation_path().clone())),
//                             )
//                         })
//                         .collect::<Vec<_>>();

//                     // Assume all non-dummy spent notes are from the same account.
//                     for (index, _) in non_dummy_spends {
//                         updater.update_spend_with(index, |mut spend_updater| {
//                             spend_updater.set_proof_generation_key(sapling_proof_gen_key.clone())
//                         })?;
//                     }

//                     Ok(())
//                 })
//                 .map_err(|e| {
//                     Error::PcztProve(format!(
//                         "Failed to add Sapling proof generation key: {:?}",
//                         e
//                     ))
//                 })?
//                 .finish()
//         } else {
//             pczt
//         };

//         let prover = LocalTxProver::bundled();
//         let pczt = Prover::new(pczt)
//             .create_orchard_proof(&orchard::circuit::ProvingKey::build())
//             .map_err(|e| Error::PcztProve(format!("Failed to create Orchard proof: {:?}", e)))?
//             .create_sapling_proofs(&prover, &prover)
//             .map_err(|e| Error::PcztProve(format!("Failed to create Sapling proofs: {:?}", e)))?
//             .finish();
//         Ok(pczt)
//     }

//     pub async fn pczt_send(&self, pczt: Pczt) -> Result<(), Error> {
//         let prover = LocalTxProver::bundled();
//         let (spend_vk, output_vk) = prover.verifying_keys();
//         let mut db = self.db.write().await;
//         let txid = extract_and_store_transaction_from_pczt::<_, ()>(
//             &mut *db,
//             pczt,
//             &spend_vk,
//             &output_vk,
//             &orchard::circuit::VerifyingKey::build(),
//         )
//         .map_err(|e| {
//             Error::PcztSend(format!(
//                 "Failed to extract and store transaction from PCZT: {:?}",
//                 e
//             ))
//         })?;
//         drop(db);
//         self.send_authorized_transactions(&NonEmpty::new(txid))
//             .await
//     }

//     pub fn pczt_combine(&self, pczts: Vec<Pczt>) -> Result<Pczt, Error> {
//         Combiner::new(pczts)
//             .combine()
//             .map_err(|e| Error::PcztCombine(format!("Failed to combine PCZT: {:?}", e)))
//     }
// }

#[cfg(test)]
mod tests {
    use zk_headstash::address::RecpAddr;
    use zk_headstash::value::NoteValue;

    use crate::wallet::MemoryHeadstashDb;

    use super::*;

    #[tokio::test]
    async fn test_headstash_wallet_creation() {
        let db = MemoryHeadstashDb::default();
        let wallet = HeadstashWallet::new(Network::TestNetwork, None)
            .await
            .unwrap();

        assert_eq!(wallet.network, Network::TestNetwork);
    }

    // #[tokio::test]
    // async fn test_store_and_list_notes() {
    //     let db = MemoryHeadstashDb::default();
    //     let wallet = HeadstashWallet::new(db, Network::TestNetwork, None)
    //         .await
    //         .unwrap();

    //     let headstash_id = "terp1contract123".to_string();
    //     let sk_bytes = [42u8; 32];
    //     let esk = EligibleSk::from_hex(&hex::encode(sk_bytes));

    //     let mut rho_bytes = [0u8; 32];
    //     getrandom::getrandom(&mut rho_bytes);
    //     let rho = Rho::from_bytes(&rho_bytes).unwrap();
    //     let recp =
    //         RecpAddr::try_from(rho_bytes.try_into().unwrap()).expect("recpAdd from rho_bytes");
    //     let value = HeadstashValue::from_raw(1000, "uterp").unwrap();

    //     // Store note
    //     wallet
    //         .gen_claim(headstash_id.clone(), esk, rho, 0, recp, value, &rho_bytes)
    //         .await
    //         .unwrap();

    //     // List unspent notes
    //     let unspent = wallet.list_unspent_notes(&headstash_id).await.unwrap();
    //     assert_eq!(unspent.len(), 1);
    //     assert_eq!(unspent[0].value.raw_amount(), 1000);
    //     assert!(!unspent[0].spent);
    // }

    // #[tokio::test]
    // async fn test_mark_note_spent() {
    //     let db = MemoryHeadstashDb::default();
    //     let wallet = HeadstashWallet::new(db, Network::TestNetwork, None)
    //         .await
    //         .unwrap();

    //     let headstash_id = "terp1contract123".to_string();
    //     let sk_bytes = [42u8; 32];
    //     let esk = EligibleSk::from_hex(&hex::encode(sk_bytes));

    //     let mut rng = rand::thread_rng();
    //     let mut rho_bytes = [0u8; 32];
    //     rand::Rng::fill_bytes(&mut rng, &mut rho_bytes);
    //     let rho = Rho::from_bytes(&rho_bytes).unwrap();

    //     let hv = HeadstashValue::from_raw(1000, "uterp").unwrap();

    //     let recp = String::default().as_bytes();
    //     // Store note
    //     wallet
    //         .gen_claim(headstash_id.clone(), esk, rho, 0, recp, hv, rng)
    //         .await
    //         .unwrap();

    //     // Get note
    //     let unspent = wallet.list_unspent_notes(&headstash_id).await.unwrap();
    //     let nullifier = unspent[0].nullifier;

    //     // Mark as spent
    //     let mut db = wallet.db.write().await;
    //     db.mark_note_spent(&headstash_id, &nullifier).unwrap();
    //     drop(db);

    //     // Verify spent
    //     let spent = wallet.list_spent_notes(&headstash_id).await.unwrap();
    //     assert_eq!(spent.len(), 1);
    //     assert!(spent[0].spent);
    // }
}
