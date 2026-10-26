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
