//! Main suite for Headstash **Product A** (private distro + claim).
//!
//! # Designed usage (default)
//!
//! | Surface | Hash | Domain |
//! |---------|------|--------|
//! | Eligibility / distro Merkle | **Poseidon-v1** | `distro_hash_domain = "poseidon-v1"` |
//! | Private note `cmx` | **Poseidon CL9** | `terp-hs-note-commit-v1` |
//! | Sinsemilla leaf / CRH | **Recovery only** | `*_sinsemilla_legacy` APIs |
//!
//! Default builders (`gen_headstash_tree`, `derive_leaf`, `merkle_crh`,
//! `generate_inclusion_test_case`, claim pre-inputs) use Poseidon. Sinsemilla
//! exports remain for historical root recovery — do not register new drops under
//! `sinsemilla-legacy`.
use alloc::boxed::Box;

use cw_orch::environment::ZkCwEnv;
#[cfg(feature = "multicore")]
use rayon::prelude::*;

use crate::{
    address::RecpAddr,
    builder::SpendInfo,
    circuit::{Circuit, ProvingKey},
    distro_poseidon::{
        poseidon_distro_crh, poseidon_distro_leaf, DistroHashDomain, DISTRO_HASH_DOMAIN_POSEIDON_V1,
    },
    keys::{EligibleSk, FullViewingKey, NullifierDerivingKey, SpendingKey},
    note::{ExtractedNoteCommitment, Note, RandomSeed, Rho},
    tree::MerklePath,
    value::{HeadstashValue, NoteDenom, NoteValue},
    Anchor, Proof, FIXED_AMOUNTS, LEAF_PERSONALIZATION, MERKLE_CRH_PERSONALIZATION,
};
use base64::{engine::general_purpose, Engine as _};
use ff::{Field, FromUniformBytes, PrimeField, PrimeFieldBits};
use hex::decode;
use pasta_curves::pallas::Base;
use pasta_curves::{arithmetic::CurveAffine, group::Curve, pallas, Fp};
use rand_core::RngCore;
use crate::os_rng;
use serde_json::{json, Value};
use sinsemilla::HashDomain;
use std::error::Error;
use std::path::{Path, PathBuf};
use std::string::{String, ToString};
use std::sync::Mutex;
use std::vec::Vec;
use std::{env, eprintln, fs, println};

// TODO: use include to paths for publishing libraries
const KEYS_DIR: &str = "artifacts";

/// BoxError
pub type BoxError = Box<dyn Error + Send + Sync>;
/// get_cli_args
pub fn get_cli_args() -> Result<(String, String), Box<dyn std::error::Error>> {
    let args: Vec<String> = env::args().collect();
    if args.len() != 3 {
        eprintln!("flag format: {} <input-file> <address>", args[0]);
        std::process::exit(1);
    }
    Ok((args[1].clone(), args[2].clone()))
}

/// TerpHeadstashConfig
#[derive(Debug)]
pub struct TerpHeadstashConfig {
    // smart contract params
    // file location params
    // storage params
    // deployment params
    // node params
}

#[cw_orch::circuit_interface(id = "headstash", artifacts_dir = "artifacts")]
#[derive(Debug, Default)]
pub struct HeadstashCircuitSuite;

/// Offline keygen: write Product A store-circuit blob for zkvm upload.
///
/// No chain / daemon required. Layout via [`crate::circuit::ProvingKey::build_and_write`]:
/// `[params][cs][vk][CircuitFooter]` — footer `i = 6`, Pasta, K=18.
pub fn build_headstash_keys_to(
    vk_path: impl AsRef<std::path::Path>,
) -> Result<crate::circuit::ProvingKey, BoxError> {
    let path = vk_path.as_ref();
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)?;
        }
    }
    eprintln!("headstash keygen → {}", path.display());
    eprintln!(
        "  Product A: Poseidon distro + Poseidon note cmx; instance width i=8; K={}",
        crate::circuit::K
    );
    let pk = crate::circuit::ProvingKey::build_and_write(path.to_path_buf())?;
    let blob = fs::read(path)?;
    validate_store_circuit_blob(&blob)?;
    eprintln!("  wrote store-circuit blob (params||cs||vk||footer) {} bytes", blob.len());
    Ok(pk)
}

/// Fail closed if the blob is not a wasmvm store-circuit body + CircuitFooter.
fn validate_store_circuit_blob(bytes: &[u8]) -> Result<(), BoxError> {
    const FOOT: usize = zk_cosmwasm::COSMWASM_FOOTER_LENGTH;
    if bytes.len() < FOOT {
        return Err(format!("store-circuit blob shorter than footer ({})", bytes.len()).into());
    }
    let footer = zk_cosmwasm::CircuitFooter::from_bytes(&bytes[bytes.len() - FOOT..])
        .map_err(|e| format!("CircuitFooter: {e}"))?;
    if footer.prover_id != Into::<u8>::into(zk_cosmwasm::CircuitType::Plonkish)
        || footer.curve_id != Into::<u8>::into(zk_cosmwasm::curves::CurveType::Pasta)
        || footer.k != crate::circuit::K as u8
        || footer.i != 8
    {
        return Err(format!(
            "footer meta expected Plonkish/Pasta/K={}/i=8, got prover={} curve={} k={} i={}",
            crate::circuit::K, footer.prover_id, footer.curve_id, footer.k, footer.i
        )
        .into());
    }
    let expect = footer.param_len as usize + footer.cs_len as usize + footer.vk_len as usize + FOOT;
    if bytes.len() != expect {
        return Err(format!(
            "blob len {} != param+cs+vk+footer {}",
            bytes.len(),
            expect
        )
        .into());
    }
    let param = &bytes[..footer.param_len as usize];
    let vk_body = &bytes[footer.param_len as usize..bytes.len() - FOOT];
    let param_hash = cosmwasm_std::Checksum::generate(param);
    let vk_hash = cosmwasm_std::Checksum::generate(vk_body);
    if param_hash.as_slice() != footer.param_checksum.as_slice() {
        return Err("param checksum mismatch vs footer".into());
    }
    if vk_hash.as_slice() != footer.vk_checksum.as_slice() {
        return Err("vk/cs checksum mismatch vs footer".into());
    }
    // Same split wasmvm uses to load appstate param + vk files.
    zk_cosmwasm::AnyVerifyingKey::from_bytes(bytes)
        .map_err(|e| format!("AnyVerifyingKey::from_bytes: {e}"))?;
    eprintln!(
        "  footer ok: Plonkish Pasta K={} i=8 param_len={} cs_len={} vk_len={}",
        footer.k, footer.param_len, footer.cs_len, footer.vk_len
    );
    eprintln!(
        "  appstate keys param={} vk={}",
        footer.param_filename(),
        footer.vk_filename()
    );
    Ok(())
}

impl<Chain: ZkCwEnv> HeadstashCircuitSuite<Chain> {
    /// Keygen for Product A claim circuit + write **store-circuit** blob for zkvm upload.
    ///
    /// Writes to the cw-orch [`CircuitUploadable::vk_path`] (default under
    /// `artifacts/`). Prefer [`build_headstash_keys_to`] for offline keygen without a live chain.
    pub fn build_keys(&self) -> Result<crate::circuit::ProvingKey, BoxError> {
        use cw_orch::prelude::CircuitUploadable;
        build_headstash_keys_to(Self::vk_path())
    }
}

impl<Chain> HeadstashBitwiseInstance for HeadstashCircuitSuite<Chain> {}
impl<Chain> HeadstashLaunchpadInstance for HeadstashCircuitSuite<Chain> {}
impl<Chain> HeadstashSinsemillaTree for HeadstashCircuitSuite<Chain> {}
impl<Chain> HeadstashIpfsInstance for HeadstashCircuitSuite<Chain> {}
impl<Chain> HeadstashTestDataGenerator for HeadstashCircuitSuite<Chain> {}

/// HeadstashBitwiseInstance
pub trait HeadstashBitwiseInstance {
    /// Derives the native pallas representation of a secp256k1 secret key.
    /// Returns `esk mod pallas_p`, matching the in-circuit `.native` value.
    fn derive_esk_native(&self, sk: [u8; 32]) -> Fp {
        use crate::spec::biguint_to_fe_simple;
        let skfq = crate::circuit::gadget::secp256k1_chip::secp_fq_from_secret_be(&sk);
        let sk_big = halo2_base::utils::fe_to_biguint(&skfq);
        biguint_to_fe_simple(&sk_big)
    }

    /// Derives the native pallas representations of the secp256k1 public key
    /// from a secret key. Computes `epk = esk * G` on secp256k1, then reduces
    /// both coordinates mod pallas_p to match the in-circuit `.native` values.
    fn derive_epk_natives(sk: [u8; 32]) -> (Fp, Fp) {
        use crate::circuit::gadget::secp256k1_chip::secp_coord_be_to_pallas_base;
        let secret_key = secp256k1::SecretKey::from_byte_array(sk).expect("valid secret key");
        let esk = EligibleSk::from(secret_key);
        let (epk_x_bytes, epk_y_bytes) = esk.epk().xy();
        // Uncompressed coords are big-endian; CRT natives use the BE integer mod pallas.
        (
            secp_coord_be_to_pallas_base(&epk_x_bytes),
            secp_coord_be_to_pallas_base(&epk_y_bytes),
        )
    }

    /// derive_v
    fn derive_v(&self, v: u64) -> [u8; 8] {
        v.to_le_bytes()
    }

    /// derive_fdi
    fn derive_fdi(&self, fdi: u64) -> [u8; 8] {
        fdi.to_le_bytes()
    }

    /// derive_nk
    fn derive_nk(&self, esk: &[u8; 32], rho: Rho) -> NullifierDerivingKey {
        NullifierDerivingKey::derive_from(
            EligibleSk::from(secp256k1::SecretKey::from_byte_array(*esk).unwrap()),
            rho,
        )
    }

    /// Convert a byte slice into an iterator of little‑endian bits (LSB first per byte).
    fn bytes_to_bits_le(bytes: &[u8]) -> impl Iterator<Item = bool> + '_ {
        bytes
            .iter()
            .flat_map(|b| (0..8).map(move |i| (b >> i) & 1 == 1))
    }

    /// Note‑Denom (nd): blake3 hash of the token, 1 bit cleared.
    fn derive_nd(raw_nd: &str) -> [u8; 32] {
        NoteDenom::new_for_proof(raw_nd)
            .as_bytes()
            .try_into()
            .expect("NoteDenom is always 32 bytes")
    }
    /// Recipient (recp): poseidon hash a 2x16byte limbs of `CanonicalAddr`
    fn derive_recp(&self, addr: [u8; 32]) -> pallas::Base {
        crate::recp_to_fp(&RecpAddr::new(addr))
    }

    /// extend_with_base_field_bits
    fn extend_with_base_field_bits(bits: &mut Vec<bool>, a: pallas::Base) {
        let bit_slice = a.to_le_bits();
        bits.extend(bit_slice.iter().take(250).map(|b| *b));
    }

    /// rho_from_secure_random
    fn rho_from_secure_random(&self) -> Rho {
        let mut randomness_64 = [0; 64];
        blake3::Hasher::new()
            .update(&headstash_randomness::ultra_secure_random())
            .finalize_xof()
            .fill(&mut randomness_64);

        Rho::from_bytes(&Base::from_uniform_bytes(&randomness_64).to_repr()).unwrap()
    }
}

/// Public inclusion / distro tree helpers for Headstash Product A.
///
/// Historical name retained for trait stability. **Defaults are Poseidon-v1**;
/// methods named `*_sinsemilla_legacy` are recovery-only (Orchard-family
/// personalizations). New drops must use Poseidon-v1 + `distro_hash_domain = "poseidon-v1"`.
pub trait HeadstashSinsemillaTree: HeadstashBitwiseInstance {
    /// `get_input_path`: cli helper to retrieve input path
    fn get_input_path(&self) -> Result<String, BoxError> {
        let args: Vec<String> = env::args().collect();
        if args.len() != 2 {
            eprintln!("Usage: {} <input-file>", args[0]);
            std::process::exit(1);
        }
        Ok(args[1].clone())
    }
    /// Find the first note matching token & amount, return its fdi
    fn print_tree(
        &self,
        input: &mut Value,
        output: Value,
        path: &std::path::Path,
    ) -> Result<(), BoxError> {
        fs::write(
            &self.get_input_path()?,
            serde_json::to_string_pretty(&input)?,
        )?;
        eprintln!("✅ Input with leaves written to {}", self.get_input_path()?);
        let merkle_path = path.join("merkle_output.json");
        fs::write(&merkle_path, serde_json::to_string_pretty(&output)?)?;
        eprintln!("✅ Merkle output written to {}", merkle_path.display());
        Ok(())
    }

    /// Generate public inclusion tree (default: **Poseidon-v1**).
    ///
    /// Alias of [`gen_headstash_tree_poseidon_v1`]. Use
    /// [`gen_headstash_tree_sinsemilla_legacy`] only for recovery fixtures.
    fn gen_headstash_tree(&self, output_path: PathBuf) -> Result<String, BoxError> {
        self.gen_headstash_tree_poseidon_v1(output_path)
    }

    /// Sinsemilla-legacy public inclusion tree (recovery only).
    fn gen_headstash_tree_sinsemilla_legacy(
        &self,
        output_path: PathBuf,
    ) -> Result<String, BoxError> {
        let mut data: Value = serde_json::from_str(&fs::read_to_string(&self.get_input_path()?)?)?;
        let mut leaves = Vec::new();
        let balances = data.as_object_mut().ok_or("Input JSON must be an object")?;

        let mut addresses: Vec<_> = balances.keys().cloned().collect();
        addresses.sort();

        for addr in addresses {
            let alloc_array = match balances.get_mut(addr.as_str()) {
                Some(v) => v,
                None => continue,
            };

            let alloc_array = match alloc_array.as_array_mut() {
                Some(arr) => arr,
                None => continue,
            };

            alloc_array.sort_by_key(|t| t["name"].to_string());

            for token in alloc_array.iter_mut() {
                let v: u64 = token["amount"]
                    .as_str()
                    .and_then(|s| s.parse::<u64>().ok())
                    .unwrap();

                let (lidxh, raw_leaves) =
                    self.derive_leaf_sinsemilla_legacy(addr.as_str(), &token["name"].to_string(), v)?;
                {
                    token
                        .as_object_mut()
                        .unwrap()
                        .entry("leaves")
                        .or_insert_with(|| json!([]));
                }

                for (fixed_amount, idx, leaf_hex) in lidxh {
                    token
                        .get_mut("leaves")
                        .unwrap()
                        .as_array_mut()
                        .unwrap()
                        .push(json!({ "amnt":fixed_amount,"index": idx, "leaf": leaf_hex }));
                }

                leaves.extend(raw_leaves);
            }
        }

        if leaves.is_empty() {
            println!("No leaves generated.");
            return Ok(String::default());
        }

        let merkle_root = self.tree_root_from_leaves_sinsemilla_legacy(leaves.clone())[0];
        let root_hex = format!("0x{}", hex::encode(merkle_root.to_repr()));
        let leaves_hex: Vec<String> = leaves
            .into_iter()
            .map(|leaf| format!("0x{}", hex::encode(leaf.to_repr())))
            .collect();

        let merkle_output = json!({
            "root": root_hex,
            "leaves": leaves_hex,
            "count": leaves_hex.len(),
            "distro_hash_domain": "sinsemilla-legacy",
        });

        self.print_tree(&mut data, merkle_output, &output_path)?;

        Ok(root_hex)
    }

    /// Helper that generates all leaves for a single token (parallelised).
    ///
    /// Default: **Poseidon-v1**.
    fn derive_leaf(
        &self,
        addr: &str,
        token_name: &str,
        v: u64,
    ) -> Result<(Vec<(u64, usize, String)>, Vec<Fp>), BoxError> {
        self.derive_leaf_poseidon_v1(addr, token_name, v)
    }

    /// Sinsemilla-legacy leaf derivation (recovery only).
    fn derive_leaf_sinsemilla_legacy(
        &self,
        addr: &str,
        token_name: &str,
        v: u64,
    ) -> Result<(Vec<(u64, usize, String)>, Vec<Fp>), BoxError> {
        // ---------- build work list ------------------------------------------------
        let mut work_items: Vec<u64> = Vec::new();
        let mut remainder = v;
        for &fixed_amount in FIXED_AMOUNTS.iter() {
            let count = remainder / fixed_amount;
            if count == 0 {
                remainder %= fixed_amount;
                continue;
            }
            // push *count* copies of the denomination value
            work_items.extend(std::iter::repeat(fixed_amount).take(count as usize));
            remainder %= fixed_amount;
        }
        debug_assert_eq!(remainder, 0, "remainder not zero after denomination split");

        // ---------- parallel leaf generation ---------------------------------------
        let leaf_hexes = Mutex::new(Vec::<(u64, usize, String)>::new());
        let raw_leaves = Mutex::new(Vec::<Fp>::new());

        let addr_bytes: &[u8; 32] = match addr.starts_with("0x") {
            true => &decode(addr.trim_start_matches("0x"))?.try_into().unwrap(),
            false => &general_purpose::STANDARD
                .decode(addr)
                .unwrap()
                .try_into()
                .unwrap(),
        };

        // `enumerate` gives us the leaf‑index (0‑based) for this address/token
        #[cfg(feature = "multicore")]
        work_items.par_iter().enumerate().try_for_each(
            |(idx, &fixed_amount)| -> Result<(), BoxError> {
                let (epk_x, epk_y) = Self::derive_epk_natives(*addr_bytes);
                let nd_fp = Fp::from_repr(Self::derive_nd(token_name)).unwrap();
                let v_fp = Fp::from(fixed_amount);
                let fdi_fp = Fp::from(idx as u64);
                let leaf = Self::leaf_hash_sinsemilla_legacy(epk_x, epk_y, nd_fp, v_fp, fdi_fp)?;
                let leaf_hex = format!("0x{}", hex::encode(leaf.to_repr()));
                leaf_hexes
                    .lock()
                    .unwrap()
                    .push((fixed_amount, idx, leaf_hex));
                raw_leaves.lock().unwrap().push(leaf);
                Ok(())
            },
        )?;

        #[cfg(not(feature = "multicore"))]
        for (idx, &fixed_amount) in work_items.iter().enumerate() {
            let (epk_x, epk_y) = Self::derive_epk_natives(*addr_bytes);
            let nd_fp = Fp::from_repr(Self::derive_nd(token_name)).unwrap();
            let v_fp = Fp::from(fixed_amount);
            let fdi_fp = Fp::from(idx as u64);
            let leaf = Self::leaf_hash_sinsemilla_legacy(epk_x, epk_y, nd_fp, v_fp, fdi_fp)?;
            let leaf_hex = format!("0x{}", hex::encode(leaf.to_repr()));
            leaf_hexes
                .lock()
                .unwrap()
                .push((fixed_amount, idx, leaf_hex));
            raw_leaves.lock().unwrap().push(leaf);
        }

        Ok((
            leaf_hexes.into_inner().unwrap(),
            raw_leaves.into_inner().unwrap(),
        ))
    }

    /// Build Merkle tree from list of leaves (default: **Poseidon-v1**).
    fn tree_root_from_leaves(&self, leaves: Vec<pallas::Base>) -> Vec<pallas::Base> {
        self.tree_root_from_leaves_poseidon_v1(leaves)
    }

    /// Sinsemilla-legacy Merkle fold (recovery only).
    fn tree_root_from_leaves_sinsemilla_legacy(
        &self,
        leaves: Vec<pallas::Base>,
    ) -> Vec<pallas::Base> {
        let mut c = leaves;
        let mut n: Vec<Fp> = Vec::new();
        let mut l = 0;

        #[cfg(feature = "multicore")]
        while c.len() > 1 {
            if c.len() % 2 != 0 {
                c.push(pallas::Base::ZERO);
            }
            let lp = l;
            let p = c
                .par_chunks(2)
                .map(|c| Self::merkle_crh_sinsemilla_legacy(lp, c[0], c[1]))
                .collect::<Vec<pallas::Base>>();
            n.extend(p);
            c = n;
            n = Vec::new();
            l += 1;
        }
        #[cfg(not(feature = "multicore"))]
        while c.len() > 1 {
            if c.len() % 2 != 0 {
                c.push(pallas::Base::ZERO);
            }
            let lp = l;
            let p = c
                .chunks(2)
                .map(|c| Self::merkle_crh_sinsemilla_legacy(lp, c[0], c[1]))
                .collect::<Vec<pallas::Base>>();
            n.extend(p);
            c = n;
            n = Vec::new();
            l += 1;
        }
        if c.is_empty() {
            vec![pallas::Base::ZERO]
        } else {
            c
        }
    }

    /// Merkle CRH (default: **Poseidon-v1**).
    fn merkle_crh(layer: u32, left: pallas::Base, right: pallas::Base) -> pallas::Base {
        Self::merkle_crh_poseidon_v1(layer, left, right)
    }

    /// Poseidon-v1 Merkle CRH for the **public inclusion** distro tree.
    ///
    /// SSOT: [`poseidon_distro_crh`]. Domain tag `terp-hs-distro-crh-v1`.
    fn merkle_crh_poseidon_v1(layer: u32, left: pallas::Base, right: pallas::Base) -> pallas::Base {
        poseidon_distro_crh(layer, left, right)
    }

    /// Sinsemilla-legacy MerkleCRH (recovery only).
    fn merkle_crh_sinsemilla_legacy(
        layer: u32,
        left: pallas::Base,
        right: pallas::Base,
    ) -> pallas::Base {
        let domain = HashDomain::new(MERKLE_CRH_PERSONALIZATION);
        let mut message = Vec::with_capacity(510);
        for i in 0..10 {
            message.push((layer >> i) & 1 == 1);
        }
        <Self as HeadstashBitwiseInstance>::extend_with_base_field_bits(&mut message, left);
        <Self as HeadstashBitwiseInstance>::extend_with_base_field_bits(&mut message, right);
        let point = domain.hash_to_point(message.into_iter()).unwrap();
        point.to_affine().coordinates().unwrap().x().clone()
    }

    /// Leaf hash (default: **Poseidon-v1**).
    fn leaf_hash(
        epk_x: Fp,
        epk_y: Fp,
        nd: Fp,
        v: Fp,
        fdi: Fp,
        rseed_com: Fp,
    ) -> Result<pallas::Base, BoxError> {
        Ok(Self::leaf_hash_poseidon_v1(
            epk_x, epk_y, nd, v, fdi, rseed_com,
        ))
    }

    /// Poseidon-v1 public inclusion **leaf** hash.
    ///
    /// Full field elements (including full `epk_y`, not Sinsemilla 1-bit packing).
    /// SSOT: [`poseidon_distro_leaf`]. Domain tag `terp-hs-distro-leaf-v1`.
    fn leaf_hash_poseidon_v1(
        epk_x: Fp,
        epk_y: Fp,
        nd: Fp,
        v: Fp,
        fdi: Fp,
        rseed_com: Fp,
    ) -> pallas::Base {
        poseidon_distro_leaf(epk_x, epk_y, nd, v, fdi, rseed_com)
    }

    /// Sinsemilla-legacy leaf (recovery only).
    ///
    /// 640-bit Sinsemilla message layout:
    ///   epk_x[0..255) || epk_y[0..1) || nd[0..255) || v[0..64) || fdi[0..64) || 0_pad
    fn leaf_hash_sinsemilla_legacy(
        epk_x: Fp,
        epk_y: Fp,
        nd: Fp,
        v: Fp,
        fdi: Fp,
    ) -> Result<pallas::Base, BoxError> {
        use ff::PrimeFieldBits;
        let mut bits: Vec<bool> = Vec::with_capacity(640);
        bits.extend(epk_x.to_le_bits().iter().by_vals().take(255));
        bits.extend(epk_y.to_le_bits().iter().by_vals().take(1));
        bits.extend(nd.to_le_bits().iter().by_vals().take(255));
        bits.extend(v.to_le_bits().iter().by_vals().take(64));
        bits.extend(fdi.to_le_bits().iter().by_vals().take(64));
        bits.push(false);
        assert_eq!(bits.len(), 640);
        Ok(HashDomain::new(LEAF_PERSONALIZATION)
            .hash_to_point(bits.into_iter())
            .expect("leaf hash should succeed")
            .to_affine()
            .coordinates()
            .unwrap()
            .x()
            .clone())
    }

    /// Build Merkle root from leaves using **Poseidon-v1** CRH (ADR default domain).
    ///
    /// Padding sibling is `ZERO` (same convention as Sinsemilla suite path and
    /// `distro_poseidon` docs). Layer `0` hashes leaves; increments toward root.
    fn tree_root_from_leaves_poseidon_v1(&self, leaves: Vec<pallas::Base>) -> Vec<pallas::Base> {
        let mut c = leaves;
        let mut n: Vec<Fp> = Vec::new();
        let mut l = 0u32;

        #[cfg(feature = "multicore")]
        while c.len() > 1 {
            if c.len() % 2 != 0 {
                c.push(pallas::Base::ZERO);
            }
            let lp = l;
            let p = c
                .par_chunks(2)
                .map(|c| Self::merkle_crh_poseidon_v1(lp, c[0], c[1]))
                .collect::<Vec<pallas::Base>>();
            n.extend(p);
            c = n;
            n = Vec::new();
            l += 1;
        }
        #[cfg(not(feature = "multicore"))]
        while c.len() > 1 {
            if c.len() % 2 != 0 {
                c.push(pallas::Base::ZERO);
            }
            let lp = l;
            let p = c
                .chunks(2)
                .map(|c| Self::merkle_crh_poseidon_v1(lp, c[0], c[1]))
                .collect::<Vec<pallas::Base>>();
            n.extend(p);
            c = n;
            n = Vec::new();
            l += 1;
        }
        if c.is_empty() {
            vec![pallas::Base::ZERO]
        } else {
            c
        }
    }

    /// Derive leaves for one address/token under **Poseidon-v1** (new Headstashes).
    fn derive_leaf_poseidon_v1(
        &self,
        addr: &str,
        token_name: &str,
        v: u64,
    ) -> Result<(Vec<(u64, usize, String)>, Vec<Fp>), BoxError> {
        let mut work_items: Vec<u64> = Vec::new();
        let mut remainder = v;
        for &fixed_amount in FIXED_AMOUNTS.iter() {
            let count = remainder / fixed_amount;
            if count == 0 {
                remainder %= fixed_amount;
                continue;
            }
            work_items.extend(std::iter::repeat(fixed_amount).take(count as usize));
            remainder %= fixed_amount;
        }
        debug_assert_eq!(remainder, 0, "remainder not zero after denomination split");

        let leaf_hexes = Mutex::new(Vec::<(u64, usize, String)>::new());
        let raw_leaves = Mutex::new(Vec::<Fp>::new());

        let addr_bytes: [u8; 32] = match addr.starts_with("0x") {
            true => decode(addr.trim_start_matches("0x"))?.try_into().unwrap(),
            false => general_purpose::STANDARD
                .decode(addr)?
                .try_into()
                .unwrap(),
        };

        #[cfg(feature = "multicore")]
        work_items.par_iter().enumerate().try_for_each(
            |(idx, &fixed_amount)| -> Result<(), BoxError> {
                let (epk_x, epk_y) = Self::derive_epk_natives(addr_bytes);
                let nd_fp = Fp::from_repr(Self::derive_nd(token_name)).unwrap();
                let v_fp = Fp::from(fixed_amount);
                let fdi_fp = Fp::from(idx as u64);
                let mut rseed = [0u8; 32];
                rseed[0] = 0x11;
                rseed[1] = idx as u8;
                let (lo, hi) = crate::claim_auth::rseed_halves(&rseed);
                let leaf = Self::leaf_hash_poseidon_v1(
                    epk_x,
                    epk_y,
                    nd_fp,
                    v_fp,
                    fdi_fp,
                    crate::claim_auth::claim_rseed_com(lo, hi),
                );
                let leaf_hex = format!("0x{}", hex::encode(leaf.to_repr()));
                leaf_hexes
                    .lock()
                    .unwrap()
                    .push((fixed_amount, idx, leaf_hex));
                raw_leaves.lock().unwrap().push(leaf);
                Ok(())
            },
        )?;

        #[cfg(not(feature = "multicore"))]
        for (idx, &fixed_amount) in work_items.iter().enumerate() {
            let (epk_x, epk_y) = Self::derive_epk_natives(addr_bytes);
            let nd_fp = Fp::from_repr(Self::derive_nd(token_name)).unwrap();
            let v_fp = Fp::from(fixed_amount);
            let fdi_fp = Fp::from(idx as u64);
            let mut rseed = [0u8; 32];
            rseed[0] = 0x11;
            rseed[1] = idx as u8;
            let (lo, hi) = crate::claim_auth::rseed_halves(&rseed);
            let leaf = Self::leaf_hash_poseidon_v1(
                epk_x,
                epk_y,
                nd_fp,
                v_fp,
                fdi_fp,
                crate::claim_auth::claim_rseed_com(lo, hi),
            );
            let leaf_hex = format!("0x{}", hex::encode(leaf.to_repr()));
            leaf_hexes
                .lock()
                .unwrap()
                .push((fixed_amount, idx, leaf_hex));
            raw_leaves.lock().unwrap().push(leaf);
        }

        Ok((
            leaf_hexes.into_inner().unwrap(),
            raw_leaves.into_inner().unwrap(),
        ))
    }

    /// Generate a **Poseidon-v1** public inclusion tree (ADR default for new drops).
    ///
    /// Writes the same JSON shape as [`gen_headstash_tree`] but roots/leaves use
    /// Poseidon-v1. Contract `distro_hash_domain` must be `poseidon-v1`.
    fn gen_headstash_tree_poseidon_v1(&self, output_path: PathBuf) -> Result<String, BoxError> {
        let mut data: Value = serde_json::from_str(&fs::read_to_string(&self.get_input_path()?)?)?;
        let mut leaves = Vec::new();
        let balances = data.as_object_mut().ok_or("Input JSON must be an object")?;

        let mut addresses: Vec<_> = balances.keys().cloned().collect();
        addresses.sort();

        for addr in addresses {
            let alloc_array = match balances.get_mut(addr.as_str()) {
                Some(v) => v,
                None => continue,
            };

            let alloc_array = match alloc_array.as_array_mut() {
                Some(arr) => arr,
                None => continue,
            };

            alloc_array.sort_by_key(|t| t["name"].to_string());

            for token in alloc_array.iter_mut() {
                let v: u64 = token["amount"]
                    .as_str()
                    .and_then(|s| s.parse::<u64>().ok())
                    .unwrap();

                let (lidxh, raw_leaves) =
                    self.derive_leaf_poseidon_v1(addr.as_str(), &token["name"].to_string(), v)?;
                {
                    token
                        .as_object_mut()
                        .unwrap()
                        .entry("leaves")
                        .or_insert_with(|| json!([]));
                }

                for (fixed_amount, idx, leaf_hex) in lidxh {
                    token
                        .get_mut("leaves")
                        .unwrap()
                        .as_array_mut()
                        .unwrap()
                        .push(json!({ "amnt":fixed_amount,"index": idx, "leaf": leaf_hex }));
                }

                leaves.extend(raw_leaves);
            }
        }

        if leaves.is_empty() {
            println!("No leaves generated.");
            return Ok(String::default());
        }

        let merkle_root = self.tree_root_from_leaves_poseidon_v1(leaves.clone())[0];
        let root_hex = format!("0x{}", hex::encode(merkle_root.to_repr()));
        let leaves_hex: Vec<String> = leaves
            .into_iter()
            .map(|leaf| format!("0x{}", hex::encode(leaf.to_repr())))
            .collect();

        let merkle_output = json!({
            "root": root_hex,
            "leaves": leaves_hex,
            "count": leaves_hex.len(),
            "distro_hash_domain": DISTRO_HASH_DOMAIN_POSEIDON_V1,
        });

        self.print_tree(&mut data, merkle_output, &output_path)?;

        Ok(root_hex)
    }

    /// create_headstash_notes
    fn create_headstash_notes(&self) -> Result<(), BoxError> {
        let (input_path, addr_target) = get_cli_args().unwrap();
        let input_data: Value = serde_json::from_str(&fs::read_to_string(&input_path)?)?;
        let mut address_notes = serde_json::Map::new();

        if let Value::Object(map) = &input_data {
            if let Some(holdings) = map.get(&addr_target) {
                if let Value::Array(holding_array) = holdings {
                    for holding in holding_array.iter() {
                        // token identifier: raw value ("uterp", "ibc/...", "tokenfactory/...")
                        let token_name = holding["name"].as_str().unwrap().to_string();
                        let _total_amount = holding["amount"].as_str().unwrap();

                        let leaves = match holding.get("leaves") {
                            Some(Value::Array(arr)) => arr,
                            _ => {
                                eprintln!("⚠️  No \"leaves\" array for token {}", token_name);
                                std::process::exit(1);
                            }
                        };

                        let mut generated_notes = Vec::new();

                        for leaf in leaves.iter() {
                            // concrete amount for this note
                            let amnt = leaf["amnt"].as_u64().unwrap_or_else(|| {
                                eprintln!("⚠️  Missing \"amnt\" in leaf for token {}", token_name);
                                std::process::exit(1);
                            });

                            // the fdi value (the leaf itself)
                            let fdi = leaf["index"].as_u64().unwrap_or_else(|| {
                                eprintln!("⚠️  Missing \"index\" in leaf for token {}", token_name);
                                std::process::exit(1);
                            });
                            generated_notes.push(json!({
                                "nd":      NoteDenom::new_for_proof(&token_name.clone()).to_string(),
                                "v":     NoteValue::from_bytes(amnt.to_le_bytes()).inner(),
                                "fdi":        fdi,
                            }));
                        }

                        // ------------------------------------------------------------------
                        // 3️⃣  Insert the array of notes for this token into the final map
                        // ------------------------------------------------------------------
                        address_notes.insert(token_name, Value::Array(generated_notes));
                    }
                } else {
                    eprintln!(
                        "Error: Address '{}' does not have holdings array.",
                        addr_target
                    );
                    std::process::exit(1);
                }
            } else {
                eprintln!("Error: Address '{}' not found in input data.", addr_target);
                std::process::exit(1);
            }
        } else {
            eprintln!("Error: Input data is not a JSON object.");
            std::process::exit(1);
        }

        // Create output file: ./data/<address>_notes.json
        let output_dir = Path::new("./data/notes");
        fs::create_dir_all(output_dir)?;
        let safe_addr: String = addr_target
            .chars()
            .filter(|c| c.is_ascii_alphanumeric())
            .collect();
        let output_path = output_dir.join(format!("{}.json", safe_addr));

        fs::write(&output_path, serde_json::to_string_pretty(&address_notes)?)?;

        eprintln!("✅ Default Genesis Notes generated for {}", addr_target);
        eprintln!("📁 Written to: {}", output_path.display());

        Ok(())
    }

    /// Client-side “my notes” export from a published tree (IPFS/API).
    ///
    /// **Product A status:** not yet wired to a live Headstash API client.
    /// Use offline suite builders instead:
    /// - tree: [`HeadstashSinsemillaTree::gen_headstash_tree_poseidon_v1`]
    /// - claim fixtures: [`HeadstashProofBuilder::suite_backed_claim_pair`]
    /// - notes JSON: [`Self::create_headstash_notes`] / related suite export paths
    fn gen_headstash_my_notes(&self, input: PathBuf, output: PathBuf) -> Result<(), BoxError> {
        let _ = (input, output);
        Err(
            "gen_headstash_my_notes: not implemented for live API — use \
             gen_headstash_tree_poseidon_v1 + suite_backed_claim_pair / create_headstash_notes \
             for Product A offline fixtures"
                .into(),
        )
    }

    /// Find the first note matching token & amount, return its fdi
    fn find_fdi(input_path: &str, token: &str, amount: &str) -> Result<u64, BoxError> {
        let json: Value =
            serde_json::from_str(&fs::read_to_string(std::path::Path::new(input_path))?)?;

        let notes = json
            .get(token)
            .and_then(|v| v.as_array())
            .ok_or("Missing or invalid `uterp` array")?;

        for note in notes {
            let denom_match = note.get("denom").and_then(|v| v.as_str()) == Some(token);
            let amount_match = note.get("amount").and_then(|v| v.as_str()) == Some(amount);

            if denom_match && amount_match {
                // fdi is a number (u32); pull it out
                let fdi = note
                    .get("fdi")
                    .and_then(|v| v.as_u64())
                    .ok_or("Missing or invalid `fdi` field")?;
                return Ok(fdi);
            }
        }

        Err(format!(
            "No note found for token '{}' with amount '{}'",
            token, amount
        )
        .into())
    }

    /// # get_note_path
    fn get_note_path() -> Result<(String, String, String), BoxError> {
        let args: Vec<String> = env::args().collect();
        if args.len() != 4 {
            eprintln!(
                "Usage: {} ./data/notes/<elig_addr> <token-denom> <amount> ",
                args[0]
            );
            std::process::exit(1);
        }
        Ok((args[1].clone(), args[2].clone(), args[3].clone()))
    }
}

// ============================================================================
// Test Data Builders for Merkle Tree Inclusion Proofs
// ============================================================================

/// Test leaf data containing all inputs needed to compute a leaf hash.
#[derive(Clone, Debug)]
pub struct TestLeafData {
    /// epk x-coordinate reduced mod pallas_p (matching in-circuit `.native`)
    pub epk_x_native: Fp,
    /// epk y-coordinate reduced mod pallas_p (matching in-circuit `.native`)
    pub epk_y_native: Fp,
    /// Note denomination as pallas field element
    pub nd: Fp,
    /// Note value as pallas field element
    pub v: Fp,
    /// Fixed denomination index as pallas field element
    pub fdi: Fp,
    /// Raw note value (u64, for building HeadstashValue)
    pub raw_v: u64,
    /// Raw denomination index (u64, for building HeadstashValue)
    pub raw_fdi: u64,
    /// Raw address bytes (for reference)
    pub raw_addr: [u8; 32],
    /// Raw token name (for reference)
    pub raw_token: String,
    /// One-time 32-byte note random. The leaf stores only its hiding commitment.
    pub rseed: [u8; 32],
}

/// Full merkle tree structure containing all levels.
/// Level 0 contains leaves, level `depth` contains the root.
#[derive(Clone, Debug)]
pub struct FullMerkleTree {
    /// All tree levels. `levels[0]` = leaves, `levels[depth]` = root (single element)
    pub levels: Vec<Vec<Fp>>,
    /// Tree depth (number of levels - 1)
    pub depth: usize,
}

impl FullMerkleTree {
    /// Get the root of the tree
    pub fn root(&self) -> Fp {
        self.levels[self.depth][0]
    }

    /// Get the number of leaves
    pub fn num_leaves(&self) -> usize {
        self.levels[0].len()
    }
}

/// Merkle authentication path for inclusion proofs.
#[derive(Clone, Debug)]
pub struct MerkleAuthPath {
    /// Sibling nodes along the path from leaf to root
    pub siblings: Vec<Fp>,
    /// Position bits indicating if node is left (0) or right (1) child at each level
    pub position_bits: Vec<bool>,
    /// Leaf index
    pub leaf_index: usize,
}

impl MerkleAuthPath {
    /// Convert to the format expected by the circuit (auth_path array)
    pub fn to_auth_path_array<const DEPTH: usize>(&self) -> [Fp; DEPTH] {
        let mut arr = [Fp::ZERO; DEPTH];
        for (i, sibling) in self.siblings.iter().enumerate().take(DEPTH) {
            arr[i] = *sibling;
        }
        arr
    }

    /// Get position as u32 (bit-packed)
    pub fn position(&self) -> u32 {
        self.position_bits
            .iter()
            .enumerate()
            .fold(0u32, |acc, (i, &bit)| acc | ((bit as u32) << i))
    }

    /// Convert MerkleAuthPath to circuit-compatible MerklePath.
    ///
    /// Pads siblings to [`MERKLE_DEPTH_ORCHARD`] (32) with `ZERO`. The on-chain /
    /// circuit **anchor** for Poseidon-v1 must be computed with
    /// [`to_circuit_path_and_root_poseidon_v1`] so the 32-layer path root matches
    /// what `Circuit::synthesize` recomputes (not the shallow suite-only root).
    pub fn to_circuit_path(&self) -> MerklePath {
        use crate::constants::MERKLE_DEPTH_ORCHARD;
        use crate::tree::MerkleHashOrchard;

        let mut auth_path_array: [MerkleHashOrchard; MERKLE_DEPTH_ORCHARD] =
            [MerkleHashOrchard::from_bytes(&pallas::Base::zero().to_repr()).unwrap();
                MERKLE_DEPTH_ORCHARD];

        for (idx, sibling) in self.siblings.iter().enumerate() {
            if idx < MERKLE_DEPTH_ORCHARD {
                auth_path_array[idx] = MerkleHashOrchard::from_bytes(&sibling.to_repr()).unwrap();
            }
        }

        MerklePath::from_parts(self.position(), auth_path_array)
    }

    /// Depth-32 Poseidon-v1 path + **circuit-consistent** root for a known leaf.
    ///
    /// Extends a variable-depth suite path with `ZERO` siblings to 32 layers and
    /// recomputes the root via [`poseidon_distro_path_root`]. Publish this root as
    /// `genesis_root` / claim `anchor` when using the Halo2 claim circuit.
    pub fn to_circuit_path_and_root_poseidon_v1(
        &self,
        leaf: Fp,
    ) -> (MerklePath, Anchor) {
        use crate::constants::MERKLE_DEPTH_ORCHARD;
        use crate::distro_poseidon::poseidon_distro_path_root;
        use crate::tree::MerkleHashOrchard;

        let path_fp: [Fp; MERKLE_DEPTH_ORCHARD] = self.to_auth_path_array();
        let position = self.position();
        let root_fp = poseidon_distro_path_root(leaf, position, &path_fp);
        let auth_path = path_fp.map(|fp| {
            MerkleHashOrchard::from_bytes(&fp.to_repr()).expect("canonical path node")
        });
        (
            MerklePath::from_parts(position, auth_path),
            Anchor::from(root_fp),
        )
    }
}

/// Poseidon-v1 **public eligibility** leaf from a fully formed claim [`Note`].
///
/// Uses the same epk native packing as suite leaf builders (BE coords → pallas).
/// Distinct from private note `cmx` ([`Note::commitment`]).
pub fn poseidon_distro_leaf_from_note(note: &Note) -> Fp {
    use crate::circuit::gadget::secp256k1_chip::secp_coord_be_to_pallas_base;
    use crate::claim_auth::{claim_rseed_com, rseed_halves};
    let (epk_x_be, epk_y_be) = note.elig_sk().epk().xy();
    let (lo, hi) = rseed_halves(note.rseed().as_bytes());
    poseidon_distro_leaf(
        secp_coord_be_to_pallas_base(&epk_x_be),
        secp_coord_be_to_pallas_base(&epk_y_be),
        note.nd().to_fp(),
        Fp::from(note.value().inner()),
        Fp::from(note.fdi()),
        claim_rseed_com(lo, hi),
    )
}

/// Partial / crafted claim note fields (pre-proof JSON + suite fixtures).
///
/// Holds the public-eligibility leaf inputs plus a recipient. Full proofs still
/// need rho/rseed and a depth-32 path; this is the "crafted partial note" surface
/// used by `create_headstash_notes` and suite-backed claim builders.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PartialClaimNote {
    /// secp256k1 eligibility secret key bytes (also suite `raw_addr` / esk).
    pub esk_bytes: [u8; 32],
    /// Token denom string (e.g. `uterp`).
    pub token: String,
    /// Claim amount.
    pub value: u64,
    /// Fixed denomination index for this leaf.
    pub fdi: u64,
    /// One-time note random. The leaf commits to it; the bytes stay off chain.
    pub rseed: [u8; 32],
    /// Raw 32-byte recipient (canonical cosmos addr bytes or test pad).
    pub recipient: [u8; 32],
}

impl PartialClaimNote {
    /// Craft from suite leaf data (eligibility key = `raw_addr`).
    pub fn from_test_leaf(data: &TestLeafData) -> Self {
        Self {
            esk_bytes: data.raw_addr,
            token: data.raw_token.clone(),
            value: data.raw_v,
            fdi: data.raw_fdi,
            rseed: data.rseed,
            recipient: data.raw_addr, // default: claim-to-self for fixtures
        }
    }

    /// Build a fully formed [`Note`] (private commit path) with matching leaf inputs.
    pub fn to_note(&self, rho: Rho, rseed: RandomSeed) -> Result<Note, BoxError> {
        let esk = EligibleSk::from_bytes(self.esk_bytes);
        let hv = HeadstashValue::from_raw(self.value, &self.token, self.fdi)?;
        let recp = RecpAddr::new(self.recipient);
        let note = Note::from_parts(hv, recp, esk, rho, rseed);
        if bool::from(note.is_some()) {
            Ok(note.unwrap())
        } else {
            Err("partial note: invalid note parts / commitment".into())
        }
    }

    /// Poseidon-v1 distro leaf field for this partial note.
    pub fn poseidon_leaf(&self) -> Fp {
        // Use the same bitwise helpers as the suite leaf builder (associated fns).
        struct Bits;
        impl HeadstashBitwiseInstance for Bits {}
        let (epk_x, epk_y) = Bits::derive_epk_natives(self.esk_bytes);
        let nd = Fp::from_repr(Bits::derive_nd(&self.token)).unwrap();
        let (lo, hi) = crate::claim_auth::rseed_halves(&self.rseed);
        poseidon_distro_leaf(
            epk_x,
            epk_y,
            nd,
            Fp::from(self.value),
            Fp::from(self.fdi),
            crate::claim_auth::claim_rseed_com(lo, hi),
        )
    }
}

/// Test data formatted for circuit consumption (Product A / Poseidon-v1).
///
/// # Shallow vs circuit root
///
/// - [`Self::root`] / [`Self::auth_path`] — suite tree depth only (path verify in suite).
/// - [`Self::circuit_anchor`] / [`Self::circuit_merkle_path`] — depth-32 path with
///   `ZERO` pad siblings + root via [`MerkleAuthPath::to_circuit_path_and_root_poseidon_v1`].
///   **These** are what `Circuit::synthesize` recomputes as public `ANCHOR`.
///
/// Never feed a shallow suite root into MockProver / on-chain claim as `anchor`
/// unless the tree was already built at depth 32.
#[derive(Clone, Debug)]
pub struct CircuitTestData {
    /// The selected leaf's input data
    pub leaf_data: TestLeafData,
    /// Computed Poseidon-v1 leaf hash
    pub leaf_hash: Fp,
    /// Authentication path at suite tree depth
    pub auth_path: MerkleAuthPath,
    /// Shallow suite root (matches `auth_path` depth only)
    pub root: Fp,
    /// Tree depth of the suite tree (not necessarily 32)
    pub tree_depth: usize,
    /// Depth-32 Merkle path for the Halo2 claim circuit
    pub circuit_merkle_path: MerklePath,
    /// Depth-32 Poseidon-v1 anchor (public instance / contract root)
    pub circuit_anchor: Anchor,
}

impl CircuitTestData {
    /// Get auth path as fixed-size array for 32-level tree (standard depth).
    pub fn auth_path_array_32(&self) -> [Fp; 32] {
        self.auth_path.to_auth_path_array()
    }

    /// Get position bits as u32.
    pub fn position(&self) -> u32 {
        self.auth_path.position()
    }

    /// Partial claim note bound to this leaf (eligibility key = `raw_addr`).
    pub fn partial_claim_note(&self) -> PartialClaimNote {
        PartialClaimNote::from_test_leaf(&self.leaf_data)
    }
}

/// Trait for generating test data for merkle tree inclusion proofs.
/// Extends `HeadstashSinsemillaTree` to provide full tree generation and path computation.
///
/// ## Usage
///
/// ```ignore
/// use zk_test_press::suite::{HeadstashCircuitSuite, MerkleTestDataBuilder};
///
/// let suite = HeadstashCircuitSuite::new(Mock::new("sender"));
///
/// // Generate test leaf data
/// let leaves_data = suite.generate_test_leaves(4);
///
/// // Compute leaf hashes
/// let leaf_hashes: Vec<_> = leaves_data.iter()
///     .map(|d| suite.compute_leaf_from_data(d).unwrap())
///     .collect();
///
/// // Build full merkle tree with all levels
/// let tree = suite.generate_full_merkle_tree(leaf_hashes.clone());
///
/// // Compute path for leaf at index 0
/// let path = suite.compute_merkle_path(&tree, 0);
///
/// // Verify the path leads to the correct root
/// assert!(suite.verify_merkle_path(&leaf_hashes[0], &path, &tree.root()));
/// ```
pub trait MerkleTestDataBuilder: HeadstashSinsemillaTree {
    /// Generate random test leaf data for testing.
    ///
    /// Creates `count` test leaves with random addresses and predetermined token/value pairs.
    fn generate_test_leaves(&self, count: usize) -> Vec<TestLeafData> {
        let mut rng = os_rng();
        let tokens = ["uterp", "ibc/ATOM", "factory/token"];
        let values = [1_000_000u64, 5_000_000u64, 10_000_000u64, 50_000_000u64];

        (0..count)
            .map(|i| {
                // Valid secp256k1 eligibility secret (also used as suite raw_addr / esk).
                let raw_addr = EligibleSk::random(&mut rng).secret_bytes();

                // Cycle through tokens and values
                let raw_token = tokens[i % tokens.len()].to_string();
                let value = values[i % values.len()];

                let (epk_x, epk_y) = Self::derive_epk_natives(raw_addr);
                TestLeafData {
                    epk_x_native: epk_x,
                    epk_y_native: epk_y,
                    nd: Fp::from_repr(Self::derive_nd(&raw_token)).unwrap(),
                    v: Fp::from(value),
                    fdi: Fp::from(i as u64),
                    raw_v: value,
                    raw_fdi: i as u64,
                    raw_addr,
                    raw_token,
                    rseed: {
                        let mut rseed = [0u8; 32];
                        rand::Rng::fill_bytes(&mut rng, &mut rseed);
                        rseed
                    },
                }
            })
            .collect()
    }

    /// Generate leaf data from specific inputs (deterministic).
    fn generate_leaf_data(
        &self,
        addr: &[u8; 32],
        token: &str,
        value: u64,
        fdi_index: u64,
    ) -> TestLeafData {
        let (epk_x, epk_y) = Self::derive_epk_natives(*addr);

        TestLeafData {
            epk_x_native: epk_x,
            epk_y_native: epk_y,
            nd: Fp::from_repr(Self::derive_nd(token)).unwrap(),
            v: Fp::from(value),
            fdi: Fp::from(fdi_index),
            raw_v: value,
            raw_fdi: fdi_index,
            raw_addr: *addr,
            raw_token: token.to_string(),
            rseed: {
                let mut rseed = [0x11; 32];
                rseed[1] = fdi_index as u8;
                rseed
            },
        }
    }

    /// Compute leaf hash from TestLeafData (default: **Poseidon-v1**).
    fn compute_leaf_from_data(&self, data: &TestLeafData) -> Result<Fp, BoxError> {
        Ok(self.compute_leaf_from_data_poseidon_v1(data))
    }

    /// Sinsemilla-legacy leaf from TestLeafData (recovery only).
    fn compute_leaf_from_data_sinsemilla_legacy(
        &self,
        data: &TestLeafData,
    ) -> Result<Fp, BoxError> {
        Self::leaf_hash_sinsemilla_legacy(
            data.epk_x_native,
            data.epk_y_native,
            data.nd,
            data.v,
            data.fdi,
        )
    }

    /// Compute **Poseidon-v1** public inclusion leaf from TestLeafData.
    fn compute_leaf_from_data_poseidon_v1(&self, data: &TestLeafData) -> Fp {
        let (lo, hi) = crate::claim_auth::rseed_halves(&data.rseed);
        <Self as HeadstashSinsemillaTree>::leaf_hash_poseidon_v1(
            data.epk_x_native,
            data.epk_y_native,
            data.nd,
            data.v,
            data.fdi,
            crate::claim_auth::claim_rseed_com(lo, hi),
        )
    }

    /// Build a full merkle tree from leaves, storing all intermediate levels.
    ///
    /// The returned tree structure contains:
    /// - `levels[0]`: Input leaves (padded to power of 2 if necessary)
    /// - `levels[i]`: Parent nodes at level i
    /// - `levels[depth]`: Single root element
    ///
    /// Full Merkle tree (default: **Poseidon-v1**).
    fn generate_full_merkle_tree(&self, leaves: Vec<Fp>) -> FullMerkleTree {
        self.generate_full_merkle_tree_poseidon_v1(leaves)
    }

    /// Sinsemilla-legacy full Merkle tree (recovery only).
    fn generate_full_merkle_tree_sinsemilla_legacy(&self, leaves: Vec<Fp>) -> FullMerkleTree {
        if leaves.is_empty() {
            return FullMerkleTree {
                levels: vec![vec![Fp::ZERO]],
                depth: 0,
            };
        }

        let mut levels: Vec<Vec<Fp>> = Vec::new();
        let mut current_level = leaves;
        if current_level.len() % 2 != 0 {
            current_level.push(Fp::ZERO);
        }
        levels.push(current_level.clone());

        let mut layer = 0u32;
        while current_level.len() > 1 {
            let mut next_level = Vec::with_capacity((current_level.len() + 1) / 2);
            for chunk in current_level.chunks(2) {
                let left = chunk[0];
                let right = if chunk.len() > 1 { chunk[1] } else { Fp::ZERO };
                let parent =
                    <Self as HeadstashSinsemillaTree>::merkle_crh_sinsemilla_legacy(layer, left, right);
                next_level.push(parent);
            }
            if next_level.len() > 1 && next_level.len() % 2 != 0 {
                next_level.push(Fp::ZERO);
            }
            levels.push(next_level.clone());
            current_level = next_level;
            layer += 1;
        }

        FullMerkleTree {
            depth: levels.len() - 1,
            levels,
        }
    }

    /// Full Merkle tree under **Poseidon-v1** CRH (new public inclusion sets).
    ///
    /// Same level layout as [`generate_full_merkle_tree`]; hashes via
    /// [`HeadstashSinsemillaTree::merkle_crh_poseidon_v1`].
    fn generate_full_merkle_tree_poseidon_v1(&self, leaves: Vec<Fp>) -> FullMerkleTree {
        if leaves.is_empty() {
            return FullMerkleTree {
                levels: vec![vec![Fp::ZERO]],
                depth: 0,
            };
        }

        let mut levels: Vec<Vec<Fp>> = Vec::new();
        let mut current_level = leaves;
        if current_level.len() % 2 != 0 {
            current_level.push(Fp::ZERO);
        }
        levels.push(current_level.clone());

        let mut layer = 0u32;
        while current_level.len() > 1 {
            let mut next_level = Vec::with_capacity((current_level.len() + 1) / 2);
            for chunk in current_level.chunks(2) {
                let left = chunk[0];
                let right = if chunk.len() > 1 { chunk[1] } else { Fp::ZERO };
                let parent =
                    <Self as HeadstashSinsemillaTree>::merkle_crh_poseidon_v1(layer, left, right);
                next_level.push(parent);
            }
            if next_level.len() > 1 && next_level.len() % 2 != 0 {
                next_level.push(Fp::ZERO);
            }
            levels.push(next_level.clone());
            current_level = next_level;
            layer += 1;
        }

        FullMerkleTree {
            depth: levels.len() - 1,
            levels,
        }
    }

    /// Compute the merkle authentication path for a leaf at a given index.
    ///
    /// Returns the path (sibling nodes) and position bits needed to verify inclusion.
    fn compute_merkle_path(&self, tree: &FullMerkleTree, leaf_index: usize) -> MerkleAuthPath {
        let mut siblings = Vec::with_capacity(tree.depth);
        let mut position_bits = Vec::with_capacity(tree.depth);
        let mut idx = leaf_index;

        for level in 0..tree.depth {
            let level_nodes = &tree.levels[level];

            // Determine if current node is left or right child
            let is_right = idx % 2 == 1;
            position_bits.push(is_right);

            // Get sibling index
            let sibling_idx = if is_right { idx - 1 } else { idx + 1 };

            // Get sibling value (zero if out of bounds)
            let sibling = if sibling_idx < level_nodes.len() {
                level_nodes[sibling_idx]
            } else {
                Fp::ZERO
            };
            siblings.push(sibling);

            // Move to parent index
            idx /= 2;
        }

        MerkleAuthPath {
            siblings,
            position_bits,
            leaf_index,
        }
    }

    /// Verify a merkle path (default: **Poseidon-v1**).
    fn verify_merkle_path(&self, leaf: &Fp, path: &MerkleAuthPath, expected_root: &Fp) -> bool {
        self.verify_merkle_path_poseidon_v1(leaf, path, expected_root)
    }

    /// Sinsemilla-legacy path verify (recovery only).
    fn verify_merkle_path_sinsemilla_legacy(
        &self,
        leaf: &Fp,
        path: &MerkleAuthPath,
        expected_root: &Fp,
    ) -> bool {
        let mut current = *leaf;
        for (level, (sibling, &is_right)) in path
            .siblings
            .iter()
            .zip(path.position_bits.iter())
            .enumerate()
        {
            let (left, right) = if is_right {
                (*sibling, current)
            } else {
                (current, *sibling)
            };
            current = <Self as HeadstashSinsemillaTree>::merkle_crh_sinsemilla_legacy(
                level as u32,
                left,
                right,
            );
        }
        current == *expected_root
    }

    /// Verify a path against a **Poseidon-v1** distro tree root.
    fn verify_merkle_path_poseidon_v1(
        &self,
        leaf: &Fp,
        path: &MerkleAuthPath,
        expected_root: &Fp,
    ) -> bool {
        let mut current = *leaf;

        for (level, (sibling, &is_right)) in path
            .siblings
            .iter()
            .zip(path.position_bits.iter())
            .enumerate()
        {
            let (left, right) = if is_right {
                (*sibling, current)
            } else {
                (current, *sibling)
            };
            current = <Self as HeadstashSinsemillaTree>::merkle_crh_poseidon_v1(
                level as u32,
                left,
                right,
            );
        }

        current == *expected_root
    }

    /// Generate a complete test case (default: **Poseidon-v1**).
    ///
    /// Returns (leaves_data, tree, selected_leaf_index, auth_path, root)
    fn generate_inclusion_test_case(
        &self,
        num_leaves: usize,
        selected_index: usize,
    ) -> Result<(Vec<TestLeafData>, FullMerkleTree, usize, MerkleAuthPath, Fp), BoxError> {
        let (leaves_data, tree, idx, path, root) =
            self.generate_inclusion_test_case_poseidon_v1(num_leaves, selected_index);
        Ok((leaves_data, tree, idx, path, root))
    }

    /// Poseidon-v1 inclusion fixture (new Headstashes / `distro_hash_domain = poseidon-v1`).
    fn generate_inclusion_test_case_poseidon_v1(
        &self,
        num_leaves: usize,
        selected_index: usize,
    ) -> (Vec<TestLeafData>, FullMerkleTree, usize, MerkleAuthPath, Fp) {
        let leaves_data = self.generate_test_leaves(num_leaves);
        let leaf_hashes: Vec<Fp> = leaves_data
            .iter()
            .map(|d| self.compute_leaf_from_data_poseidon_v1(d))
            .collect();
        let tree = self.generate_full_merkle_tree_poseidon_v1(leaf_hashes);
        let path = self.compute_merkle_path(&tree, selected_index);
        let root = tree.root();
        (leaves_data, tree, selected_index, path, root)
    }

    /// Generate Product A test data for the claim circuit.
    ///
    /// Includes both the **shallow** suite path/root (for suite path unit tests)
    /// and the **depth-32** `circuit_merkle_path` / `circuit_anchor` that match
    /// `distro_poseidon_gadget::calculate_distro_root_poseidon`.
    ///
    /// Prefer [`HeadstashProofBuilder::suite_backed_claim_pair`] when you need a
    /// full `(Circuit, Instance)` ready for MockProver.
    fn generate_circuit_test_data(
        &self,
        num_leaves: usize,
        selected_index: usize,
    ) -> Result<CircuitTestData, BoxError> {
        let (leaves_data, tree, idx, path, root) =
            self.generate_inclusion_test_case(num_leaves, selected_index)?;

        let selected_leaf = &leaves_data[idx];
        let leaf_hash = self.compute_leaf_from_data(selected_leaf)?;
        let (circuit_merkle_path, circuit_anchor) =
            path.to_circuit_path_and_root_poseidon_v1(leaf_hash);

        Ok(CircuitTestData {
            leaf_data: selected_leaf.clone(),
            leaf_hash,
            auth_path: path,
            root,
            tree_depth: tree.depth,
            circuit_merkle_path,
            circuit_anchor,
        })
    }
}

// Implement MerkleTestDataBuilder for HeadstashCircuitSuite
impl<Chain> MerkleTestDataBuilder for HeadstashCircuitSuite<Chain> {}

/// All actions any user would take for creating a new headstash 100% client side using this launchpad framework.
///  Requires struct implementing trait to also implement `HeadstashBitwiseInstance` default members.
/// TODO: feature flag parallelization in tree generation
/// TODO: add default documentation to each member
pub trait HeadstashIpfsInstance: HeadstashBitwiseInstance {
    /// `upload_circuit_keys`: upload keys to ipfs for public distribution
    fn upload_circuit_keys(&self) -> Result<(), BoxError> {
        // check for existing ipfs connection
        // options:
        // -  use node local ipfs gateway
        // -  use remote ipfs gateway
        // -  deploy new one if needed
        // load key files from default folder
        // upload and handle response gracefully
        Ok(())
    }
    /// `upload_headstash_params`: upload headstash params to ipfs for public distribution
    fn upload_headstash_yaml(&self) -> Result<(), BoxError> {
        Ok(())
    }
    /// `req_headstash_pk`: request headstash proof keys from storage method defined by params
    fn req_headstash_pk(&self) -> Result<ProvingKey, BoxError> {
        todo!()
    }
}

// /// Trait for generating headstash circuit keys in binary format.
// /// Provides methods to build, write, and load circuit proving/verifying keys.
// pub trait CircuitKeysGenerator {
//     /// Circuit size parameter (K = 18 for headstash)
//     const HEADSTASH_K: u32 = 18;
//     /// Generate headstash circuit keys and write to directory.
//     /// Creates: verifying_key.bin, proving_key.bin (which includes params)
//     fn gen_circuit_keys(
//         &self,
//         base_path: &Path,
//     ) -> Result<(crate::circuit::VerifyingKey, crate::circuit::ProvingKey), BoxError> {
//         fs::create_dir_all(&base_path)?;
//         let pk = crate::circuit::ProvingKey::build();
//         info!("Writing keys to {:?}...", base_path);
//         let vk_path = base_path.join(VK_FILE);
//         let mut vk_file = std::fs::File::create(&vk_path)?;
//         vk.params.write(&mut vk_file)?;
//         vk.vk.write(&mut vk_file)?;
//         println!("  Written: {:?}", vk_path);
//         let pk_path = base_path.join(PK_FILE);
//         crate::circuit::ProvingKey::build_and_write(pk_path.clone())?;
//         println!("  Written: {:?}", pk_path);

//         Ok((vk, pk))
//     }

//     /// Get the default keys directory path.
//     fn keys_dir(&self) -> PathBuf {
//         PathBuf::from(KEYS_DIR)
//     }
// }

// impl<Chain> CircuitKeysGenerator for HeadstashCircuitSuite<Chain> {}

/// Proof bundle containing proof and public inputs for a headstash claim.
#[derive(Clone, Debug)]
pub struct HeadstashProofBundle {
    /// The generated proof (using native headstash circuit proof type)
    pub proof: crate::Proof,
    /// Public inputs (anchor, nd, v, recp, nf, cmx)
    pub instance: crate::circuit::Instance,
    /// The merkle tree anchor
    pub anchor: Anchor,
}

/// Trait for building headstash proofs from genesis distribution data.
/// Optimized for 1-time-spend model (static merkle tree, single claim per note).
pub trait HeadstashProofBuilder: HeadstashBitwiseInstance + MerkleTestDataBuilder {
    /// Build SpendInfo from note and suite auth path (Product A / Poseidon-v1).
    ///
    /// Pads to depth 32 and uses the **eligibility distro leaf** derived from the
    /// note (not private `cmx`) so the path matches claim-circuit inclusion.
    fn build_spend_info(
        &self,
        note: &Note,
        fvk: &FullViewingKey,
        auth_path: &MerkleAuthPath,
    ) -> Result<SpendInfo, BoxError> {
        let leaf = poseidon_distro_leaf_from_note(note);
        let (merkle_path, _anchor) = auth_path.to_circuit_path_and_root_poseidon_v1(leaf);
        SpendInfo::new(fvk.clone(), note.clone(), merkle_path)
            .ok_or_else(|| "Failed to create SpendInfo".into())
    }

    /// Create a headstash proof from test leaf data.
    ///
    /// Uses the **same** eligibility key as the suite leaf (`raw_addr` → esk), a
    /// depth-32 Poseidon-v1 path root as `anchor`, and
    /// `from_action_context_unchecked` for 1-time spend semantics.
    fn create_genesis_proof_from_leaf(
        &self,
        pk: &crate::circuit::ProvingKey,
        leaf_data: &TestLeafData,
        auth_path: &MerkleAuthPath,
    ) -> Result<HeadstashProofBundle, BoxError> {
        let mut rng = os_rng();

        let sk = SpendingKey::random(&mut rng);
        let fvk = FullViewingKey::from(&sk);

        // Eligibility key MUST match the distro leaf (suite stores sk in raw_addr).
        let esk = EligibleSk::from_bytes(leaf_data.raw_addr);
        let secret_be = esk.secret_bytes();

        let rho = self.rho_from_secure_random();
        let rseed = RandomSeed::from_bytes(leaf_data.rseed, &rho).expect("rseed issue");

        let partial = PartialClaimNote::from_test_leaf(leaf_data);
        let leaf = partial.poseidon_leaf();
        let (merkle_path, anchor) = auth_path.to_circuit_path_and_root_poseidon_v1(leaf);

        let hv =
            HeadstashValue::from_raw(leaf_data.raw_v, &leaf_data.raw_token, leaf_data.raw_fdi)?;
        let recp = RecpAddr::new(leaf_data.raw_addr);
        let note = Note::from_parts(hv, recp, esk, rho, rseed).expect("correct note parts");

        let spend_info = SpendInfo::new(fvk, note.clone(), merkle_path)
            .ok_or("SpendInfo creation failed")?;

        let nf = note.nullifier();
        let cmx = ExtractedNoteCommitment::from(note.commitment());
        let mut instance =
            crate::circuit::Instance::from_parts(anchor, hv.denom(), hv.amount(), recp, nf, cmx);

        let mut circuit = Circuit::from_action_context_unchecked(spend_info, note);
        circuit.attach_personal_sign(&mut instance, &secret_be);
        let proof = Proof::create(pk, &[circuit], &[instance.clone()], &mut rng)?;

        Ok(HeadstashProofBundle {
            proof,
            instance,
            anchor,
        })
    }

    /// Suite-backed multi-leaf claim: circuit + instance ready for MockProver / prove.
    ///
    /// Builds a Poseidon-v1 tree of `num_leaves`, selects `selected_index`, pads the
    /// auth path to depth 32, and constructs a note whose (epk, nd, v, fdi) match the
    /// suite leaf so genesis inclusion and note commit stay consistent.
    fn suite_backed_claim_pair(
        &self,
        num_leaves: usize,
        selected_index: usize,
    ) -> Result<(Circuit, crate::circuit::Instance, Anchor, PartialClaimNote), BoxError> {
        let (leaves_data, _tree, idx, path, _shallow_root) =
            self.generate_inclusion_test_case_poseidon_v1(num_leaves, selected_index);
        let leaf_data = &leaves_data[idx];
        let partial = PartialClaimNote::from_test_leaf(leaf_data);
        let leaf = partial.poseidon_leaf();
        let (merkle_path, anchor) = path.to_circuit_path_and_root_poseidon_v1(leaf);

        let mut rng = os_rng();
        let sk = SpendingKey::random(&mut rng);
        let fvk = FullViewingKey::from(&sk);
        let esk = EligibleSk::from_bytes(leaf_data.raw_addr);
        let secret_be = esk.secret_bytes();
        let rho = self.rho_from_secure_random();
        let rseed = RandomSeed::from_bytes(leaf_data.rseed, &rho).expect("rseed");

        let hv =
            HeadstashValue::from_raw(leaf_data.raw_v, &leaf_data.raw_token, leaf_data.raw_fdi)?;
        let recp = RecpAddr::new(leaf_data.raw_addr);
        let note = Note::from_parts(hv, recp, esk, rho, rseed).expect("note");

        let spend_info =
            SpendInfo::new(fvk, note.clone(), merkle_path).ok_or("SpendInfo failed")?;
        let mut circuit = Circuit::from_action_context_unchecked(spend_info, note.clone());

        let nf = note.nullifier();
        let cmx = ExtractedNoteCommitment::from(note.commitment());
        let mut instance =
            crate::circuit::Instance::from_parts(anchor, hv.denom(), hv.amount(), recp, nf, cmx);
        circuit.attach_personal_sign(&mut instance, &secret_be);

        Ok((circuit, instance, anchor, partial))
    }

    // /// Verify a headstash proof.
    // fn verify_genesis_proof(
    //     &self,
    //     vk: &crate::circuit::VerifyingKey,
    //     bundle: &HeadstashProofBundle,
    // ) -> Result<(), BoxError> {
    //     bundle
    //         .proof
    //         .verify(vk, &[bundle.instance.clone()])
    //         .map_err(|e| format!("Proof verification failed: {:?}", e).into())
    // }
}

impl<Chain> HeadstashProofBuilder for HeadstashCircuitSuite<Chain> {}

/// launchpad
pub trait HeadstashLaunchpadInstance: HeadstashBitwiseInstance + HeadstashIpfsInstance {
    /// ## [create_headstash_proof]
    /// > #### Default method for creating a proof. Requires both the *public (instance)* & *private (witnesses)* values.
    fn create_headstash_proof(
        &self,
        a: Anchor,
        mp: MerklePath,
        esk: EligibleSk,
        recp: RecpAddr,
        hv: HeadstashValue,
    ) -> Result<Proof, BoxError> {
        let mut rng = os_rng();
        let r = self.rho_from_secure_random().to_bytes();
        let rho = self.rho_from_secure_random();
        let rseed = RandomSeed::from_bytes(r, &rho).expect("random seed");

        let pk = self.req_headstash_pk()?;
        let spk = SpendingKey::from_bytes(r).expect("spk");
        let fvk = FullViewingKey::from(&spk);

        let secret_be = esk.secret_bytes();
        let n = Note::from_parts(hv, recp, esk, rho, rseed).expect("note derivation");
        let nf = n.nullifier();
        let cmx = ExtractedNoteCommitment::from(n.commitment());

        let c = SpendInfo::new(fvk, n.clone(), mp).expect("headstash claim");

        // generate proof, unchecked as we rho is not deterministically derived
        let mut instances =
            crate::circuit::Instance::from_parts(a, hv.denom(), hv.amount(), recp, nf, cmx);
        let mut circuit = Circuit::from_action_context_unchecked(c, n);
        circuit.attach_personal_sign(&mut instances, &secret_be);
        Ok(Proof::create(&pk, &[circuit], &[instances], &mut rng)?)
    }
    /// create_new_headstash
    fn create_new_headstash(&self) -> Result<(), BoxError> {
        // generate template headstash yaml
        self.gen_new_headstash_params();
        // prompt to determine communities to include in headstash airdrop
        // deploy/retrieve holder distributions via full ephemeral full nodes api queries
        self.gen_community_snapshots();
        // prompt calculations on percentile distribution and suggested ranges for normalization of airdrop allocation between communities
        self.gen_calculate_distribution();
        // generate headstash circuit
        // self.gen_headstash_circuit()?;
        // upload circuit keys to ipfs
        self.upload_circuit_keys()?;
        // upload headstash yaml to ipfs
        self.upload_headstash_yaml()?;
        // call headstash launchpad
        // deploy new headstash aggregator
        unimplemented!()
    }

    /// `gen_headstgen_community_snapshotsash_keys`: retrive snapshot and pubkeys of list of community holders.
    fn gen_new_headstash_params(&self) {
        // load config file or create new one
        // a. determine what circuit keys used
        //  - default headstash, custom one we upload
        // b. smart contract params
        // c. deployment params
        // d. node params
    }

    /// `gen_headstgen_community_snapshotsash_keys`: retrive snapshot and pubkeys of list of community holders.
    fn gen_community_snapshots(&self) {
        // load config file
        // connect to eth node
        // retrieve latest holder distribution and pubkeys for each community
        // write csv into each community folder
    }
    /// `gen_calculate_distribution`:  .
    fn gen_calculate_distribution(&self) {}
}

// ============================================================================
// E2E Test Data Generator
// ============================================================================

/// Complete E2E test bundle containing all generated artifacts.
#[derive(Debug)]
pub struct HeadstashE2ETestBundle {
    /// Circuit verifying key
    pub vk: crate::circuit::VerifyingKey,
    /// Circuit proving key
    pub pk: crate::circuit::ProvingKey,
    /// Full merkle tree
    pub tree: FullMerkleTree,
    /// Test leaf data for all accounts
    pub leaves: Vec<TestLeafData>,
    /// Per-account test data with proofs
    pub accounts: Vec<HeadstashAccountTestData>,
}

/// Per-account test data including keys, proof, and instance.
#[derive(Clone, Debug)]
pub struct HeadstashAccountTestData {
    /// Account index in the tree
    pub index: usize,
    /// Spending key bytes (for test reproducibility)
    pub sk_bytes: [u8; 32],
    /// Eligible secret key
    pub esk: EligibleSk,
    /// The generated proof
    pub proof: crate::Proof,
    /// Public instance
    pub instance: crate::circuit::Instance,
    /// Merkle authentication path
    pub auth_path: MerkleAuthPath,
    /// Computed anchor
    pub anchor: Anchor,
    /// Whether the merkle path was verified valid
    pub path_valid: bool,
}

/// Trait for generating complete E2E test data for headstash circuits.
/// Consolidates all test generation logic into the suite for reuse across projects.
pub trait HeadstashTestDataGenerator:
    HeadstashBitwiseInstance + MerkleTestDataBuilder + HeadstashProofBuilder
{
    // /// Generate a complete E2E test bundle with circuit keys, merkle tree, and proofs.
    // ///
    // /// This is the canonical method for generating headstash test data.
    // /// Returns all artifacts needed for E2E testing.
    // fn generate_e2e_test_bundle(
    //     &self,
    //     num_accounts: usize,
    // ) -> Result<HeadstashE2ETestBundle, BoxError> {
    //     let mut rng = OsRng;

    //     // Step 1: Build circuit keys
    //     eprintln!("[1/4] Building circuit keys (K=17)...");
    //     let pk = self.build_keys()?;
    //     let vk = pk.vk();

    //     eprintln!("  Circuit keys built");

    //     // Step 2: Generate test leaves
    //     eprintln!("[2/4] Generating {} test leaves...", num_accounts);
    //     let leaves = self.generate_test_leaves(num_accounts);

    //     // Compute leaf hashes
    //     let leaf_hashes: Vec<Fp> = leaves
    //         .iter()
    //         .map(|leaf| self.compute_leaf_from_data(leaf).expect("leaf hash"))
    //         .collect();

    //     // Step 3: Build merkle tree
    //     eprintln!("[3/4] Building merkle tree...");
    //     let tree = self.generate_full_merkle_tree(leaf_hashes.clone());
    //     eprintln!("  Tree depth: {}", tree.depth);

    //     // Step 4: Generate proofs for each account
    //     eprintln!("[4/4] Generating proofs for {} accounts...", num_accounts);
    //     let mut accounts = Vec::with_capacity(num_accounts);

    //     for i in 0..num_accounts {
    //         eprintln!("  Account {}/{}...", i + 1, num_accounts);
    //         let auth_path = self.compute_merkle_path(&tree, i);
    //         let path_valid = self.verify_merkle_path(&leaf_hashes[i], &auth_path, &tree.root());
    //         let merkle_path = auth_path.to_circuit_path();
    //         let leaf = &leaves[i];
    //         // Generate random keys
    //         let mut sk_bytes = [0u8; 32];
    //         rng.fill_bytes(&mut sk_bytes);
    //         let sk = SpendingKey::from_bytes(sk_bytes).expect("valid spending key");
    //         let fvk = FullViewingKey::from(&sk);
    //         let esk = EligibleSk::random(&mut rng);
    //         // Create rho and rseed
    //         let rho = self.rho_from_secure_random();
    //         let mut rseed_bytes = [0u8; 32];
    //         rng.fill_bytes(&mut rseed_bytes);
    //         let rseed = RandomSeed::from_bytes(rseed_bytes, &rho).expect("valid rseed");

    //         // Build HeadstashValue
    //         let hv = HeadstashValue::from_raw(leaf.raw_v, &leaf.raw_token, leaf.raw_fdi)?;

    //         // Create note
    //         let recp = RecpAddr::new(leaf.raw_addr);
    //         let note = Note::from_parts(hv, recp, esk.clone(), rho, rseed).expect("note creation");

    //         // Compute anchor and build instance
    //         let anchor = merkle_path.root(note.commitment().into());
    //         let nf = note.nullifier();
    //         let cmx = ExtractedNoteCommitment::from(note.commitment());
    //         let instance = crate::circuit::Instance::from_parts(
    //             anchor,
    //             hv.denom(),
    //             hv.amount(),
    //             recp,
    //             nf,
    //             cmx,
    //         );

    //         // Build circuit and generate proof
    //         let spend_info = SpendInfo::new(fvk, note.clone(), merkle_path).expect("SpendInfo");
    //         let circuit = Circuit::from_action_context_unchecked(spend_info, note);
    //         let proof = Proof::create(&pk, &[circuit], &[instance.clone()], &mut rng)?;

    //         // Verify proof
    //         proof.verify(&vk, &[instance.clone()])?;

    //         accounts.push(HeadstashAccountTestData {
    //             index: i,
    //             sk_bytes,
    //             esk,
    //             proof,
    //             instance,
    //             auth_path,
    //             anchor,
    //             path_valid,
    //         });
    //     }

    //     Ok(HeadstashE2ETestBundle {
    //         vk,
    //         pk,
    //         tree,
    //         leaves,
    //         accounts,
    //     })
    // }

    /// Write E2E test bundle to files in the specified directory.
    ///
    /// Creates:
    /// - `headstash.vk.bin` - Verifying key binary
    /// - `tree.json` - Merkle tree metadata
    /// - `headstash_test_data.json` - Full test data with private keys
    /// - `headstash_proofs.json` - Simplified proofs for E2E scripts
    fn write_e2e_test_files(
        &self,
        bundle: &HeadstashE2ETestBundle,
        output_dir: &Path,
    ) -> Result<(), BoxError> {
        use std::io::Write;

        std::fs::create_dir_all(output_dir)?;

        // Write verifying key
        let vk_path = output_dir.join("headstash.vk.bin");
        let mut vk_file = std::fs::File::create(&vk_path)?;
        bundle.vk.params.write(&mut vk_file)?;
        bundle.vk.vk.write(&mut vk_file)?;

        // Write tree metadata
        let tree_meta = json!({
            "root": format!("{:?}", bundle.tree.root()),
            "depth": bundle.tree.depth,
            "num_leaves": bundle.tree.num_leaves(),
        });
        let tree_path = output_dir.join("tree.json");
        std::fs::File::create(&tree_path)?
            .write_all(serde_json::to_string_pretty(&tree_meta)?.as_bytes())?;

        // Build account data
        let mut accounts_json = Vec::new();
        for (account, leaf) in bundle.accounts.iter().zip(bundle.leaves.iter()) {
            let (epkx, epky) = account.esk.epk().xy();

            accounts_json.push(json!({
                "account_index": account.index,
                "private_keys": {
                    "spending_key": general_purpose::STANDARD.encode(&account.sk_bytes),
                    "eligible_sk": general_purpose::STANDARD.encode(account.esk.secret_bytes()),
                },
                "public_keys": {
                    "epk_x": general_purpose::STANDARD.encode(&epkx),
                    "epk_y": general_purpose::STANDARD.encode(&epky),
                },
                "leaf_data": {
                    "epk_x_native": format!("{:?}", leaf.epk_x_native),
                    "epk_y_native": format!("{:?}", leaf.epk_y_native),
                    "nd": format!("{:?}", leaf.nd),
                    "v": format!("{:?}", leaf.v),
                    "fdi": format!("{:?}", leaf.fdi),
                    "raw_token": leaf.raw_token.clone(),
                },
                "merkle": {
                    "position": account.auth_path.position(),
                    "tree_depth": bundle.tree.depth,
                    "leaf_index": account.index,
                    "anchor": format!("{:?}", account.anchor),
                },
                "instance": self.instance_to_json(&account.instance),
                "proof": general_purpose::STANDARD.encode(account.proof.as_ref()),
                "path_valid": account.path_valid,
            }));
        }

        // Write full test data
        let full_output = json!({
            "generated_at": format!("{:?}", std::time::SystemTime::now()),
            "num_accounts": bundle.accounts.len(),
            "circuit_k": 17,
            "vk_path": "headstash.vk.bin",
            "tree": tree_meta,
            "accounts": accounts_json,
        });
        let json_path = output_dir.join("headstash_test_data.json");
        std::fs::File::create(&json_path)?
            .write_all(serde_json::to_string_pretty(&full_output)?.as_bytes())?;

        // Write simplified proofs file
        let mut proofs_map = serde_json::Map::new();
        for account in &bundle.accounts {
            proofs_map.insert(
                format!("account_{}", account.index),
                json!({
                    "proof": general_purpose::STANDARD.encode(account.proof.as_ref()),
                    "instance": self.instance_to_json(&account.instance),
                }),
            );
        }
        let proofs_path = output_dir.join("headstash_proofs.json");
        std::fs::File::create(&proofs_path)?
            .write_all(serde_json::to_string_pretty(&Value::Object(proofs_map))?.as_bytes())?;

        Ok(())
    }

    /// Convert Instance to JSON for serialization.
    fn instance_to_json(&self, instance: &crate::circuit::Instance) -> Value {
        json!({
            "anchor": format!("{:?}", instance.anchor),
            "nd": format!("{:?}", instance.nd),
            "v": instance.v.inner(),
            "recp": format!("{:?}", instance.recp),
            "nf": format!("{:?}", instance.nf),
            "cmx": format!("{:?}", instance.cmx),
        })
    }
}

// TODO:
// - notecommitment derivation accuracy
// - nullifier derivation accuracy
// - document DST & hashing algo constant in spec

// TEST:
// nullifier should not be impacted by randomness inputs
// nullifier should change with different esk/epk
//

#[cfg(test)]
mod test {
    use cw_orch::mock::Mock;

    use super::*;
    use std::boxed::Box;
    use std::collections::HashMap;

    #[test]
    pub fn test_note_accuracy() -> Result<(), Box<dyn std::error::Error>> {
        // // Load original allocations
        // let input_data: Value =
        //     serde_json::from_str(&fs::read_to_string("./data/genesis_sinsemilla.json")?)?;

        // // Load generated notes for the zero address
        // let notes_path = "./data/notes/0x0000000000000000000000000000000000000000.json";
        // let calculated_notes: Value = serde_json::from_str(&fs::read_to_string(notes_path)?)?;

        // // Extract original holdings
        // let mut original_balances: HashMap<String, u64> = HashMap::new();

        // if let Value::Object(map) = &input_data {
        //     if let Some(holdings) = map.get("0x0000000000000000000000000000000000000000") {
        //         if let Value::Array(holding_array) = holdings {
        //             for holding in holding_array {
        //                 if let Some(name) = holding["name"].as_str() {
        //                     let amount_str = holding["amount"].as_str().unwrap_or("0");
        //                     let amount: u64 = amount_str.parse().unwrap_or(0);
        //                     *original_balances.entry(name.to_string()).or_insert(0) += amount;
        //                 }
        //             }
        //         }
        //     }
        // }

        // // Extract and sum note values from generated notes
        // let mut notes_sum: HashMap<String, u64> = HashMap::new();

        // if let Value::Object(note_map) = &calculated_notes {
        //     for (token_name, notes) in note_map {
        //         if let Value::Array(note_array) = notes {
        //             for note in note_array {
        //                 if let Some(v_str) = note["v"].as_str() {
        //                     let v: u64 = v_str.parse().unwrap_or(0);
        //                     *notes_sum.entry(token_name.clone()).or_insert(0) += v;
        //                 }
        //             }
        //         }
        //     }
        // }

        // // Compare: original vs summed note values
        // for (token, original_amount) in &original_balances {
        //     let note_total = notes_sum.get(token).copied().unwrap_or(0);
        //     assert_eq!(
        //         original_amount, &note_total,
        //         "Token {}: allocation ({}) does not match total notes ({})",
        //         token, original_amount, note_total
        //     );
        // }

        // // Also check for extra tokens in notes not in original
        // for (token, _) in &notes_sum {
        //     assert!(
        //         original_balances.contains_key(token),
        //         "Token {} appears in notes but not in original allocation",
        //         token
        //     );
        // }

        Ok(())
    }

    // fn load_data() -> Result<Value, BoxError> {
    //     let file = fs::File::open("./data/genesis_sinsemilla.json")?;
    //     let reader = std::io::BufReader::new(file);
    //     Ok(serde_json::from_reader(reader)?)
    // }

    // // Ensures input data is in compatible format
    // #[test]
    // pub fn test_input_data_accuracy() -> Result<(), BoxError> {
    //     let data = load_data()?;
    //     if !data.is_object() {
    //         panic!("Expected JSON object (map) at root");
    //     }

    //     for (addr, tokens) in data.as_object().unwrap().iter() {
    //         assert!(tokens.is_array(), "Value for {} must be an array", addr);
    //         for token in tokens.as_array().unwrap() {
    //             let obj = token.as_object().unwrap();
    //             assert!(obj.contains_key("amount"), "missing required key 'amount'");
    //             assert!(obj.contains_key("token"), " missing required key 'token'");
    //             assert!(obj["amount"].is_string(), "'amount' must be a string");
    //             assert!(obj["token"].is_string(), "'token' must be a string");
    //         }
    //     }

    //     Ok(())
    // }

    // #[test]
    // pub fn test_leaves_accuracy() -> Result<(), BoxError> {
    //     let data = load_data()?;
    //     // Expect top-level object: { "addr": [ { token, amount, leaf }, ... ] }
    //     let balances = data.as_object().ok_or("JSON must be an object")?;
    //     for (address, allocs) in balances {
    //         let alloc_array = allocs.as_array().unwrap();

    //         for token_obj in alloc_array {
    //             let token_name = token_obj["token"].as_str().unwrap_or_default();
    //             let amount = token_obj["amount"].as_str().unwrap_or_default();
    //             let expected_leaf_hex = token_obj["leaf"].as_str().unwrap_or_default();

    //             // Remove 0x prefix if present
    //             let expected_bytes = if expected_leaf_hex.starts_with("0x") {
    //                 hex::decode(&expected_leaf_hex[2..])?
    //             } else {
    //                 hex::decode(expected_leaf_hex)?
    //             };

    //             // Re-compute expected scalar from address + token + amount
    //             let computed_leaf = leaf_hash(address, token_name, amount,)?;
    //             let computed_bytes = computed_leaf.to_repr();

    //             // Compare raw field element bytes
    //             assert_eq!(
    //                 computed_bytes.as_ref(),
    //                 expected_bytes.as_slice(),
    //                 "Leaf mismatch for address={}, token={}",
    //                 address,
    //                 token_name
    //             );
    //         }
    //     }

    //     Ok(())
    // }

    // =========================================================================
    // MerkleTestDataBuilder Tests
    // =========================================================================

    #[test]
    fn test_generate_test_leaves() {
        let suite = HeadstashCircuitSuite::new(Mock::new("sender"));
        let leaves = suite.generate_test_leaves(4);

        assert_eq!(leaves.len(), 4);
        for (i, leaf) in leaves.iter().enumerate() {
            // epk_x_native and epk_y_native are Fp field elements (always valid)
            // nd is now an Fp field element
            // v is now an Fp field element
            // fdi is now an Fp field element
            // fdi should match index
            assert_eq!(leaf.fdi, Fp::from(i as u64));
        }
    }

    #[test]
    fn test_full_merkle_tree_single_leaf() {
        let suite = HeadstashCircuitSuite::new(Mock::new("sender"));
        let leaf = Fp::from(42u64);

        let tree = suite.generate_full_merkle_tree(vec![leaf]);

        // Single leaf padded to 2, then hashed to root
        assert!(tree.levels.len() >= 2);
        assert_eq!(tree.levels[0].len(), 2); // padded
        assert_eq!(tree.levels[tree.depth].len(), 1); // root
    }

    #[test]
    fn test_full_merkle_tree_four_leaves() {
        let suite = HeadstashCircuitSuite::new(Mock::new("sender"));
        let leaves: Vec<Fp> = (0..4).map(|i| Fp::from(i as u64)).collect();

        let tree = suite.generate_full_merkle_tree(leaves.clone());

        assert_eq!(tree.levels[0].len(), 4);
        assert_eq!(tree.levels[1].len(), 2);
        assert_eq!(tree.levels[2].len(), 1);
        assert_eq!(tree.depth, 2);
    }

    #[test]
    fn test_merkle_path_computation() {
        let suite = HeadstashCircuitSuite::new(Mock::new("sender"));
        let leaves: Vec<Fp> = (0..4).map(|i| Fp::from(i as u64)).collect();

        let tree = suite.generate_full_merkle_tree(leaves.clone());

        for i in 0..4 {
            let path = suite.compute_merkle_path(&tree, i);
            assert_eq!(path.siblings.len(), tree.depth);
            assert_eq!(path.position_bits.len(), tree.depth);
            assert_eq!(path.leaf_index, i);
        }
    }

    #[test]
    fn test_merkle_path_verification() {
        let suite = HeadstashCircuitSuite::new(Mock::new("sender"));
        let leaves: Vec<Fp> = (0..8).map(|i| Fp::from(i as u64)).collect();

        let tree = suite.generate_full_merkle_tree(leaves.clone());
        let root = tree.root();

        // Verify path for each leaf
        for (i, leaf) in leaves.iter().enumerate() {
            let path = suite.compute_merkle_path(&tree, i);
            assert!(
                suite.verify_merkle_path(leaf, &path, &root),
                "Path verification failed for leaf {}",
                i
            );
        }
    }

    #[test]
    fn test_merkle_path_verification_fails_wrong_leaf() {
        let suite = HeadstashCircuitSuite::new(Mock::new("sender"));
        let leaves: Vec<Fp> = (0..4).map(|i| Fp::from(i as u64)).collect();

        let tree = suite.generate_full_merkle_tree(leaves.clone());
        let root = tree.root();

        // Use path for leaf 0 but try to verify with wrong leaf
        let path = suite.compute_merkle_path(&tree, 0);
        let wrong_leaf = Fp::from(999u64);

        assert!(
            !suite.verify_merkle_path(&wrong_leaf, &path, &root),
            "Verification should fail with wrong leaf"
        );
    }

    #[test]
    fn test_generate_inclusion_test_case() {
        let suite = HeadstashCircuitSuite::new(Mock::new("sender"));
        let result = suite.generate_inclusion_test_case(8, 3);
        assert!(result.is_ok());

        let (leaves_data, tree, idx, path, root) = result.unwrap();

        assert_eq!(leaves_data.len(), 8);
        assert_eq!(idx, 3);

        // Verify the path is valid
        let leaf_hash = suite.compute_leaf_from_data(&leaves_data[3]).unwrap();
        assert!(suite.verify_merkle_path(&leaf_hash, &path, &root));
    }

    #[test]
    fn test_poseidon_v1_leaf_differs_from_sinsemilla_legacy() {
        let suite = HeadstashCircuitSuite::new(Mock::new("sender"));
        let data = suite.generate_leaf_data(&[7u8; 32], "uterp", 1_000_000, 0);
        let sin = suite.compute_leaf_from_data_sinsemilla_legacy(&data).unwrap();
        let pos = suite.compute_leaf_from_data_poseidon_v1(&data);
        assert_ne!(sin, pos, "domains must not collide");
        // Default compute path is Poseidon-v1.
        assert_eq!(suite.compute_leaf_from_data(&data).unwrap(), pos);
        assert_eq!(DistroHashDomain::default().as_str(), DISTRO_HASH_DOMAIN_POSEIDON_V1);
    }

    #[test]
    fn test_poseidon_v1_merkle_path_verification() {
        let suite = HeadstashCircuitSuite::new(Mock::new("sender"));
        let (leaves_data, _tree, idx, path, root) =
            suite.generate_inclusion_test_case_poseidon_v1(8, 3);

        assert_eq!(leaves_data.len(), 8);
        assert_eq!(idx, 3);

        let leaf_hash = suite.compute_leaf_from_data_poseidon_v1(&leaves_data[3]);
        assert!(
            suite.verify_merkle_path_poseidon_v1(&leaf_hash, &path, &root),
            "Poseidon-v1 path must recompute to registered root"
        );
        // Default verify path is Poseidon-v1 (same).
        assert!(suite.verify_merkle_path(&leaf_hash, &path, &root));
        // Sinsemilla-legacy verifier must not accept a Poseidon-v1 root.
        assert!(!suite.verify_merkle_path_sinsemilla_legacy(&leaf_hash, &path, &root));
    }

    #[test]
    fn test_poseidon_v1_two_leaf_matches_distro_module() {
        let suite = HeadstashCircuitSuite::new(Mock::new("sender"));
        let sk0 = EligibleSk::random(&mut rand::rng()).secret_bytes();
        let sk1 = EligibleSk::random(&mut rand::rng()).secret_bytes();
        let d0 = suite.generate_leaf_data(&sk0, "uterp", 1_000_000, 0);
        let d1 = suite.generate_leaf_data(&sk1, "uterp", 5_000_000, 1);
        let l0 = suite.compute_leaf_from_data_poseidon_v1(&d0);
        let l1 = suite.compute_leaf_from_data_poseidon_v1(&d1);
        let root_suite = suite.tree_root_from_leaves_poseidon_v1(vec![l0, l1])[0];
        let root_ssot = poseidon_distro_crh(0, l0, l1);
        assert_eq!(root_suite, root_ssot);
    }

    #[test]
    fn test_partial_claim_note_and_depth32_root() {
        let suite = HeadstashCircuitSuite::new(Mock::new("sender"));
        let (leaves, _tree, idx, path, shallow) =
            suite.generate_inclusion_test_case_poseidon_v1(8, 2);
        let partial = PartialClaimNote::from_test_leaf(&leaves[idx]);
        let leaf = partial.poseidon_leaf();
        assert_eq!(leaf, suite.compute_leaf_from_data_poseidon_v1(&leaves[idx]));
        let (_mp, anchor) = path.to_circuit_path_and_root_poseidon_v1(leaf);
        // Depth-32 extended root differs from shallow suite root when depth < 32.
        if path.siblings.len() < 32 {
            assert_ne!(anchor.to_bytes(), shallow.to_repr());
        }
        // Path recompute is stable.
        let (_mp2, anchor2) = path.to_circuit_path_and_root_poseidon_v1(leaf);
        assert_eq!(anchor.to_bytes(), anchor2.to_bytes());
    }

    #[test]
    fn test_suite_backed_claim_pair_constructs() {
        let suite = HeadstashCircuitSuite::new(Mock::new("sender"));
        let (circuit, instance, anchor, partial) =
            suite.suite_backed_claim_pair(4, 1).expect("claim pair");
        assert_eq!(instance.v.inner(), partial.value);
        assert_eq!(instance.anchor.to_bytes(), anchor.to_bytes());
        // Witnesses assigned (depth-32 auth path present as known values).
        let mut saw_path = false;
        circuit.path.map(|_| saw_path = true);
        assert!(saw_path, "circuit path witness must be known");
    }

    #[test]
    fn test_zkvm_circuit_public_layout() {
        // Spec for CosmwasmCircuit / CircuitFooter public inputs (build_and_write).
        use crate::circuit::{Instance, K};
        assert_eq!(K, 18, "zkvm circuit K");
        // Instance wire: 6 * 32 = 168 bytes (anchor, nd, v, recp, nf, cmx).
        // Footer `i` field in ProvingKey::build_and_write is 6.
        let (circuit, instance, _, _) = HeadstashCircuitSuite::new(Mock::new("sender"))
            .suite_backed_claim_pair(2, 0)
            .unwrap();
        assert_eq!(instance.to_bytes().len(), 168);
        let rows = instance.to_halo2_instance();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].len(), 9); // padded halo2 instance column width
        let _ = circuit;
    }

    #[test]
    fn test_circuit_test_data_generation() {
        let suite = HeadstashCircuitSuite::new(Mock::new("sender"));
        let result = suite.generate_circuit_test_data(4, 1);
        assert!(result.is_ok());

        let circuit_data = result.unwrap();

        assert!(circuit_data.tree_depth > 0);
        assert_eq!(circuit_data.auth_path.leaf_index, 1);

        // Shallow suite path verifies against shallow root.
        assert!(suite.verify_merkle_path(
            &circuit_data.leaf_hash,
            &circuit_data.auth_path,
            &circuit_data.root
        ));

        // Depth-32 circuit path root is the product-facing anchor.
        let (mp2, anchor2) = circuit_data
            .auth_path
            .to_circuit_path_and_root_poseidon_v1(circuit_data.leaf_hash);
        assert_eq!(
            circuit_data.circuit_anchor.to_bytes(),
            anchor2.to_bytes(),
            "CircuitTestData.circuit_anchor must match to_circuit_path_and_root_poseidon_v1"
        );
        assert_eq!(
            mp2.position(),
            circuit_data.circuit_merkle_path.position()
        );
        // When suite depth < 32, shallow root ≠ circuit anchor (pad layers).
        if circuit_data.tree_depth < 32 {
            assert_ne!(
                circuit_data.circuit_anchor.to_bytes(),
                circuit_data.root.to_repr(),
                "shallow suite root must not be used as circuit/on-chain anchor when depth < 32"
            );
        }

        let partial = circuit_data.partial_claim_note();
        assert_eq!(partial.poseidon_leaf(), circuit_data.leaf_hash);
    }

    #[test]
    fn test_circuit_test_data_and_suite_backed_shapes() {
        // Independent OsRng trees share Product A tooling shape, not the same root.
        let suite = HeadstashCircuitSuite::new(Mock::new("sender"));
        let data = suite.generate_circuit_test_data(8, 2).unwrap();
        let (circuit, instance, anchor, partial) =
            suite.suite_backed_claim_pair(8, 2).unwrap();

        assert_eq!(instance.anchor.to_bytes(), anchor.to_bytes());
        assert_eq!(instance.to_bytes().len(), 168);
        assert_eq!(data.auth_path.leaf_index, 2);

        let rebuilt = suite.generate_leaf_data(
            &partial.esk_bytes,
            &partial.token,
            partial.value,
            partial.fdi,
        );
        assert_eq!(
            partial.poseidon_leaf(),
            suite.compute_leaf_from_data_poseidon_v1(&rebuilt)
        );
        assert_eq!(
            data.leaf_hash,
            suite.compute_leaf_from_data_poseidon_v1(&data.leaf_data)
        );
        let mut saw = false;
        circuit.path.map(|_| saw = true);
        assert!(saw, "suite_backed_claim_pair must assign depth-32 path");
        assert_ne!(data.circuit_anchor.to_bytes(), [0u8; 32]);
    }

    #[test]
    fn test_deterministic_leaf_generation() {
        let suite = HeadstashCircuitSuite::new(Mock::new("sender"));
        // Must be a valid secp256k1 scalar (not all-zero).
        let mut addr = [7u8; 32];
        addr[0] = 1;
        let token = "uterp";
        let value = 1_000_000u64;
        let fdi = 0u64;

        let leaf1 = suite.generate_leaf_data(&addr, token, value, fdi);
        let leaf2 = suite.generate_leaf_data(&addr, token, value, fdi);

        assert_eq!(leaf1.epk_x_native, leaf2.epk_x_native);
        assert_eq!(leaf1.epk_y_native, leaf2.epk_y_native);
        assert_eq!(leaf1.nd, leaf2.nd);
        assert_eq!(leaf1.v, leaf2.v);
        assert_eq!(leaf1.fdi, leaf2.fdi);

        let hash1 = suite.compute_leaf_from_data(&leaf1).unwrap();
        let hash2 = suite.compute_leaf_from_data(&leaf2).unwrap();
        assert_eq!(hash1, hash2);
    }

    #[test]
    #[cfg(feature = "multicore")]
    fn test_tree_root_consistency_with_existing() {
        let suite = HeadstashCircuitSuite::new(Mock::new("sender"));
        let leaves: Vec<Fp> = (0..4).map(|i| Fp::from(i as u64)).collect();

        // Compare with existing tree_root_from_leaves (requires multicore feature)
        let existing_root = suite.tree_root_from_leaves(leaves.clone())[0];
        let full_tree = suite.generate_full_merkle_tree(leaves);

        assert_eq!(
            existing_root,
            full_tree.root(),
            "Full tree root should match existing implementation"
        );
    }

    #[test]
    fn test_position_encoding() {
        let suite = HeadstashCircuitSuite::new(Mock::new("sender"));
        let leaves: Vec<Fp> = (0..8).map(|i| Fp::from(i as u64)).collect();
        let tree = suite.generate_full_merkle_tree(leaves);

        // Leaf 0: position = 0b000 = 0
        let path0 = suite.compute_merkle_path(&tree, 0);
        assert_eq!(path0.position(), 0);

        // Leaf 1: position = 0b001 = 1
        let path1 = suite.compute_merkle_path(&tree, 1);
        assert_eq!(path1.position(), 1);

        // Leaf 2: position = 0b010 = 2
        let path2 = suite.compute_merkle_path(&tree, 2);
        assert_eq!(path2.position(), 2);

        // Leaf 5: position = 0b101 = 5
        let path5 = suite.compute_merkle_path(&tree, 5);
        assert_eq!(path5.position(), 5);
    }

    #[test]
    fn test_auth_path_array_conversion() {
        let suite = HeadstashCircuitSuite::new(Mock::new("sender"));
        let leaves: Vec<Fp> = (0..4).map(|i| Fp::from(i as u64)).collect();
        let tree = suite.generate_full_merkle_tree(leaves);

        let path = suite.compute_merkle_path(&tree, 0);
        let arr: [Fp; 32] = path.to_auth_path_array();

        // First elements should match siblings
        for (i, sibling) in path.siblings.iter().enumerate() {
            assert_eq!(arr[i], *sibling);
        }
        // Remaining elements should be zero
        for i in path.siblings.len()..32 {
            assert_eq!(arr[i], Fp::ZERO);
        }
    }

    #[test]
    fn test_larger_tree() {
        let suite = HeadstashCircuitSuite::new(Mock::new("sender"));
        let leaves: Vec<Fp> = (0..64).map(|i| Fp::from(i as u64)).collect();

        let tree = suite.generate_full_merkle_tree(leaves.clone());
        let root = tree.root();

        // 64 leaves -> 6 levels (2^6 = 64)
        assert_eq!(tree.depth, 6);

        // Verify all paths
        for (i, leaf) in leaves.iter().enumerate() {
            let path = suite.compute_merkle_path(&tree, i);
            assert!(
                suite.verify_merkle_path(leaf, &path, &root),
                "Path verification failed for leaf {} in 64-leaf tree",
                i
            );
        }
    }

    #[test]
    fn test_odd_number_of_leaves() {
        let suite = HeadstashCircuitSuite::new(Mock::new("sender"));
        // 5 leaves (odd number)
        let leaves: Vec<Fp> = (0..5).map(|i| Fp::from(i as u64)).collect();

        let tree = suite.generate_full_merkle_tree(leaves.clone());
        let root = tree.root();

        // Padded to 6 leaves at level 0
        assert_eq!(tree.levels[0].len(), 6);

        // Verify paths for original leaves
        for (i, leaf) in leaves.iter().enumerate() {
            let path = suite.compute_merkle_path(&tree, i);
            assert!(
                suite.verify_merkle_path(leaf, &path, &root),
                "Path verification failed for leaf {} in odd-leaf tree",
                i
            );
        }
    }

    #[test]
    fn test_merkle_auth_path_to_circuit_path() {
        let suite = HeadstashCircuitSuite::new(Mock::new("sender"));

        // Generate a small tree
        let test_leaves = suite.generate_test_leaves(4);
        let leaf_hashes: Vec<_> = test_leaves
            .iter()
            .map(|l| suite.compute_leaf_from_data(l).unwrap())
            .collect();
        let tree = suite.generate_full_merkle_tree(leaf_hashes);

        // Get auth path and convert
        let auth_path = suite.compute_merkle_path(&tree, 1);
        let circuit_path = auth_path.to_circuit_path();

        // Verify position matches
        assert_eq!(auth_path.position(), circuit_path.position());

        // Verify auth path length is MERKLE_DEPTH_ORCHARD (32)
        assert_eq!(
            circuit_path.auth_path().len(),
            crate::constants::MERKLE_DEPTH_ORCHARD
        );

        // Verify first siblings match (before padding)
        for (i, sibling) in auth_path.siblings.iter().enumerate() {
            let circuit_sibling = circuit_path.auth_path()[i];
            assert_eq!(
                sibling.to_repr(),
                circuit_sibling.to_bytes(),
                "Sibling mismatch at index {}",
                i
            );
        }
    }

    // #[test]
    // #[ignore] // Expensive test - run with: cargo test test_headstash_e2e_proof_generation -- --ignored
    // fn test_headstash_e2e_proof_generation() -> Result<(), BoxError> {
    //     let suite = HeadstashCircuitSuite::new(Mock::new("sender"));

    //     // Step 1: Generate circuit keys
    //     println!("Generating circuit keys...");
    //     let pk = suite.build_keys()?;

    //     // Step 2: Generate test merkle tree
    //     let num_leaves = 4;
    //     let test_leaves = suite.generate_test_leaves(num_leaves);
    //     let leaf_hashes: Vec<_> = test_leaves
    //         .iter()
    //         .map(|l| suite.compute_leaf_from_data(l).unwrap())
    //         .collect();
    //     let tree = suite.generate_full_merkle_tree(leaf_hashes.clone());

    //     // Step 3: Generate and verify proof for first leaf
    //     let auth_path = suite.compute_merkle_path(&tree, 0);

    //     // Verify merkle path is valid
    //     assert!(suite.verify_merkle_path(&leaf_hashes[0], &auth_path, &tree.root()));

    //     // Create proof
    //     let proof_bundle =
    //         suite.create_genesis_proof_from_leaf(&pk, &test_leaves[0], &auth_path)?;

    //     // Verify proof
    //     // suite.verify_genesis_proof(&vk, &proof_bundle)?;

    //     println!("E2E proof generation and verification successful!");
    //     Ok(())
    // }
}
