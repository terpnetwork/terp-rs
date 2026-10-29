//! Browser (wasm-bindgen) entry for the K=11 membership prover used by the
//! Headstash claim page. The witness never leaves the worker.
//! Build: tools/headstash/scripts/build-browser-prover.sh

use std::boxed::Box;
use std::string::{String, ToString};
use std::vec::Vec;

use ff::{Field, PrimeField};
use pasta_curves::pallas;
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = console)]
    fn error(msg: &str);
}

pub(crate) fn phase(msg: &str) {
    error(msg);
}

#[wasm_bindgen(start)]
pub fn install_panic_hook() {
    std::panic::set_hook(Box::new(|info| {
        error(&info.to_string());
    }));
    std::alloc::set_alloc_error_hook(|layout| {
        let msg = String::from("alloc-fail size=")
            + &layout.size().to_string()
            + " align="
            + &layout.align().to_string();
        error(&msg);
    });
}



fn wasm_mem() -> usize {
    #[cfg(target_arch = "wasm32")]
    {
        core::arch::wasm32::memory_size(0) * 65536
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        0
    }
}

/// Prove the no-ECDSA membership fixture at the measured minimum K.
/// Keygen runs inside this call. The fixture secret is the native test
/// value `Base::from(0x11)` and is not returned.
#[wasm_bindgen]
pub fn prove_membership_browser(destination: &str) -> Result<String, JsError> {
    phase(&format!("membership-enter mem={}", wasm_mem()));
    let secret = pallas::Base::from(0x11u64);
    let ctx = pallas::Base::from(0x22u64);
    let path = [pallas::Base::ZERO; crate::no_ecdsa::MEMBERSHIP_DEPTH];
    let proof = crate::no_ecdsa::prove_membership(secret, ctx, 15, 0, path, destination)
        .map_err(|err| JsError::new(&err))?;
    phase(&format!(
        "membership-exit mem={} k={} proof_bytes={}",
        wasm_mem(),
        proof.k,
        proof.proof.len()
    ));
    let body = serde_json::json!({
        "k": proof.k,
        "proof_hex": hex::encode(&proof.proof),
        "proof_bytes": proof.proof.len(),
        "pk_bytes": proof.pk_bytes,
        "body": proof.body,
        "destination": destination,
        "amount": 15,
    });
    Ok(serde_json::to_string(&body).map_err(|err| JsError::new(&err.to_string()))?)
}

/// Little-endian hex of Poseidon(terp-hs-keycom-v1, secret).
#[wasm_bindgen]
pub fn membership_key_com_hex(secret_hex: &str) -> Result<String, JsError> {
    let bytes = hex::decode(secret_hex).map_err(|_| JsError::new("secret"))?;
    if bytes.len() != 32 {
        return Err(JsError::new("secret"));
    }
    let mut repr = [0u8; 32];
    repr.copy_from_slice(&bytes);
    let secret = Option::from(pallas::Base::from_repr(repr)).ok_or_else(|| JsError::new("secret"))?;
    Ok(hex::encode(crate::no_ecdsa::key_commitment(secret).to_repr()))
}

/// Prove a real membership witness. JSON fields are secret_hex, ctx_hex,
/// root_hex, amount, index, siblings_hex (16), and destination.
#[wasm_bindgen]
pub fn prove_membership_witness_browser(witness_json: &str) -> Result<String, JsError> {
    phase(&format!("witness-enter mem={}", wasm_mem()));
    let proof = crate::no_ecdsa::prove_witness_json(witness_json).map_err(|err| JsError::new(&err))?;
    phase(&format!(
        "witness-exit mem={} k={} proof_bytes={}",
        wasm_mem(),
        proof.k,
        proof.proof.len()
    ));
    let body = serde_json::json!({
        "k": proof.k,
        "vk_id": crate::no_ecdsa::MEMBERSHIP_VK_ID,
        "proof_hex": hex::encode(&proof.proof),
        "proof_bytes": proof.proof.len(),
        "pk_bytes": proof.pk_bytes,
        "body": proof.body,
    });
    Ok(serde_json::to_string(&body).map_err(|err| JsError::new(&err.to_string()))?)
}

/// vk_id 3 note prover (depth 17, denomination leaf, pinned tags). Same JSON
/// witness shape as `prove_membership_witness_browser` with 17 siblings and an
/// amount that must be one of the 12 denomination classes.
#[wasm_bindgen]
pub fn prove_note_witness_browser(witness_json: &str) -> Result<String, JsError> {
    phase(&format!("note-enter mem={}", wasm_mem()));
    let proof = crate::note_membership::prove_note_witness_json(witness_json)
        .map_err(|err| JsError::new(&err))?;
    phase(&format!("note-exit mem={} k={} proof_bytes={}", wasm_mem(), proof.k, proof.proof.len()));
    let body = serde_json::json!({
        "vk_id": crate::note_membership::NOTE_VK_ID,
        "k": proof.k,
        "proof_hex": hex::encode(&proof.proof),
        "proof_bytes": proof.proof.len(),
        "pk_bytes": proof.pk_bytes,
        "body": proof.body,
    });
    Ok(serde_json::to_string(&body).map_err(|err| JsError::new(&err.to_string()))?)
}

/// Sound raw-key claim (secp256k1 sk·G in-circuit, fixed-base comb) proved in the browser.
/// Params + keygen run in the client (IPA: no trusted setup). Fixture uses a test-only sk.
/// Returns JSON: k, proof_len, pk_bytes, mem (wasm linear memory, bytes) per phase.
#[wasm_bindgen]
pub fn prove_rawkey_fixture_browser(k: u32, destination: &str) -> Result<String, JsError> {
    use halo2_proofs::{
        plonk,
        poly::commitment::Params,
        transcript::{Blake2bWrite, Challenge255},
    };
    use pasta_curves::vesta;
    use crate::rawkey::blind_k19::blind_fixture;
    use crate::note_membership::NOTE_DEPTH;
    let e = |s: String| JsError::new(&s);
    phase(&format!("rawkey-enter mem={}", wasm_mem()));
    let mut path = [pallas::Base::ZERO; NOTE_DEPTH];
    for (i, p) in path.iter_mut().enumerate() {
        *p = pallas::Base::from(1000 + i as u64);
    }
    let fx = blind_fixture([0x11u8; 32], pallas::Base::from(7u64), 1234, 5, path, destination).map_err(e)?;
    phase(&format!("rawkey-fixture mem={}", wasm_mem()));
    let params = Params::<vesta::Affine>::new(k);
    phase(&format!("rawkey-params mem={}", wasm_mem()));
    let vk = plonk::keygen_vk(&params, &fx.circuit).map_err(|x| JsError::new(&format!("vk {x:?}")))?;
    phase(&format!("rawkey-vk mem={}", wasm_mem()));
    let pk = plonk::keygen_pk(&params, vk, &fx.circuit).map_err(|x| JsError::new(&format!("pk {x:?}")))?;
    let mut pk_buf = Vec::new();
    pk.write(&mut pk_buf).map_err(|x| JsError::new(&x.to_string()))?;
    let pk_bytes = pk_buf.len();
    drop(pk_buf);
    phase(&format!("rawkey-pk mem={} pk_bytes={}", wasm_mem(), pk_bytes));
    let mut rng = crate::os_rng();
    let mut tr = Blake2bWrite::<_, vesta::Affine, Challenge255<_>>::init(Vec::new());
    plonk::create_proof(&params, &pk, &[fx.circuit], &[&[&fx.public]], &mut rng, &mut tr)
        .map_err(|x| JsError::new(&format!("prove {x:?}")))?;
    let proof = tr.finalize();
    phase(&format!("rawkey-proved mem={} proof={}", wasm_mem(), proof.len()));
    crate::no_ecdsa::verify_membership(&params, pk.get_vk(), &proof, &fx.public).map_err(e)?;
    phase(&format!("rawkey-verified mem={}", wasm_mem()));
    Ok(format!(
        "{{\"k\":{},\"proof_len\":{},\"pk_bytes\":{},\"mem\":{}}}",
        k,
        proof.len(),
        pk_bytes,
        wasm_mem()
    ))
}

// ---- k19 lab task 3: multithreaded wasm + PK cache -------------------------------------------

/// rayon threads visible to halo2 (1 without `browser-mt`).
#[wasm_bindgen]
pub fn rawkey_threads() -> usize {
    #[cfg(feature = "browser-mt")]
    return rayon::current_num_threads();
    #[cfg(not(feature = "browser-mt"))]
    1
}

fn rawkey_fx(destination: &str) -> Result<crate::rawkey::blind_k19::BlindFixture, JsError> {
    use crate::note_membership::NOTE_DEPTH;
    let mut path = [pallas::Base::ZERO; NOTE_DEPTH];
    for (i, p) in path.iter_mut().enumerate() {
        *p = pallas::Base::from(1000 + i as u64);
    }
    crate::rawkey::blind_k19::blind_fixture([0x11u8; 32], pallas::Base::from(7u64), 1234, 5, path, destination)
        .map_err(|s| JsError::new(&s))
}

/// Run params + keygen once and serialize: [u32 len][params][u32 len][cs][pk] (IndexedDB/download blob).
#[wasm_bindgen]
pub fn rawkey_keygen_artifacts(k: u32) -> Result<Vec<u8>, JsError> {
    use halo2_proofs::{plonk, poly::commitment::Params};
    use pasta_curves::vesta;
    let fx = rawkey_fx("terp1lab")?;
    let params = Params::<vesta::Affine>::new(k);
    phase("art-params");
    let vk = plonk::keygen_vk(&params, &fx.circuit).map_err(|x| JsError::new(&format!("vk {x:?}")))?;
    phase("art-vk");
    let pk = plonk::keygen_pk(&params, vk, &fx.circuit).map_err(|x| JsError::new(&format!("pk {x:?}")))?;
    phase("art-pk");
    let je = |x: std::io::Error| JsError::new(&x.to_string());
    let (mut pb, mut cb, mut out) = (Vec::new(), Vec::new(), Vec::new());
    params.write(&mut pb).map_err(je)?;
    pk.get_vk().cs().write(&mut cb).map_err(je)?;
    out.extend_from_slice(&(pb.len() as u32).to_le_bytes());
    out.extend_from_slice(&pb);
    out.extend_from_slice(&(cb.len() as u32).to_le_bytes());
    out.extend_from_slice(&cb);
    pk.write(&mut out).map_err(je)?;
    phase(&format!("art-serialized bytes={} params={} cs={}", out.len(), pb.len(), cb.len()));
    Ok(out)
}

/// Load cached artifacts (no keygen), prove + verify. Phase logs: load-params, load-pk, proved, verified.
#[wasm_bindgen]
pub fn prove_rawkey_cached(artifacts: &[u8], destination: &str) -> Result<String, JsError> {
    use halo2_proofs::{plonk, poly::commitment::Params, transcript::{Blake2bWrite, Challenge255}};
    use pasta_curves::vesta;
    let bad = || JsError::new("artifacts truncated");
    let rd = |b: &[u8], o: usize| -> Option<usize> { Some(u32::from_le_bytes(b.get(o..o + 4)?.try_into().ok()?) as usize) };
    let pl = rd(artifacts, 0).ok_or_else(bad)?;
    let mut r = artifacts.get(4..4 + pl).ok_or_else(bad)?;
    let je = |x: std::io::Error| JsError::new(&x.to_string());
    let params = Params::<vesta::Affine>::read(&mut r).map_err(je)?;
    phase(&format!("load-params mem={}", wasm_mem()));
    let o = 4 + pl;
    let cl = rd(artifacts, o).ok_or_else(bad)?;
    let mut cr = artifacts.get(o + 4..o + 4 + cl).ok_or_else(bad)?;
    let cs = plonk::ConstraintSystem::<pallas::Base>::read(&mut cr).map_err(je)?;
    let mut pr = artifacts.get(o + 4 + cl..).ok_or_else(bad)?;
    let pk = plonk::ProvingKey::<vesta::Affine>::read_with_cs(&mut pr, &params, cs).map_err(je)?;
    phase(&format!("load-pk mem={}", wasm_mem()));
    let fx = rawkey_fx(destination)?;
    let mut rng = crate::os_rng();
    let mut tr = Blake2bWrite::<_, vesta::Affine, Challenge255<_>>::init(Vec::new());
    plonk::create_proof(&params, &pk, &[fx.circuit], &[&[&fx.public]], &mut rng, &mut tr)
        .map_err(|x| JsError::new(&format!("prove {x:?}")))?;
    let proof = tr.finalize();
    phase(&format!("proved mem={} proof={}", wasm_mem(), proof.len()));
    crate::no_ecdsa::verify_membership(&params, pk.get_vk(), &proof, &fx.public).map_err(|s| JsError::new(&s))?;
    phase(&format!("verified mem={}", wasm_mem()));
    Ok(format!("{{\"proof_len\":{},\"threads\":{},\"mem\":{}}}", proof.len(), rawkey_threads(), wasm_mem()))
}
