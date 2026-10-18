//! Loopback headstash lane. Binds only 127.0.0.1.
//! `--prove` writes a claim JSON and exits. Otherwise it serves /ve/*.
//! Custody seed is a file of 32 bytes. The process prints neither the seed
//! nor the verifying-key hash.

use std::path::PathBuf;

use ed25519_dalek::SigningKey;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let bind = std::env::var("HASH_MARKET_BIND").unwrap_or_else(|_| "127.0.0.1:18521".into());
    if !bind.starts_with("127.0.0.1:") {
        anyhow::bail!("refusing non-loopback bind");
    }
    std::env::set_var("HASHMERCHANT_CHAIN_UID", "headstash-claim");
    eprintln!("lane_install");
    let hash = hash_market::ve::vm_verify::install_epoch_host()
        .map_err(|err| anyhow::anyhow!(err))?;
    if std::env::args().any(|arg| arg == "--prove") {
        let recipient = std::env::var("HEADSTASH_RECIPIENT")?;
        let out = std::env::var("HEADSTASH_PROVE_OUT")?;
        let ctx = [7u8; 32];
        let raw = headstash_claim::statement::decode_destination("terp", &recipient)
            .map_err(|_| anyhow::anyhow!("recipient"))?;
        let mut padded = [0u8; 32];
        padded[..20].copy_from_slice(&raw);
        let bundle = headstash_epoch::prove_epoch_claim(&recipient, padded, 15, ctx)
            .map_err(|err| anyhow::anyhow!(err))?;
        let body = serde_json::json!({
            "ctx_hex": hex::encode(bundle.ctx),
            "root_hex": hex::encode(bundle.root),
            "nullifier_hex": hex::encode(bundle.nullifier),
            "denom_hex": hex::encode(denom_bytes()),
            "destination": bundle.destination,
            "amount": bundle.amount,
            "message": bundle.message,
            "proof_hex": hex::encode(&bundle.proof),
            "vk_id": 1,
        });
        std::fs::write(&out, serde_json::to_vec(&body)?)?;
        hash_market::owner_file::write_owner_only(
            &PathBuf::from(&out).with_extension("vk"),
            base64_vk(&hash).as_bytes(),
        )?;
        println!("prove_ok bytes {}", bundle.proof.len());
        return Ok(());
    }
    let dir = PathBuf::from(std::env::var("HASH_MARKET_DATA").unwrap_or_else(|_| "/tmp/hm-lane".into()));
    std::fs::create_dir_all(&dir)?;
    hash_market::owner_file::write_owner_only(&dir.join("vk.hash"), base64_vk(&hash).as_bytes())?;
    let signer = load_signer()?;
    let public = signer.verifying_key().to_bytes();
    hash_market::owner_file::write_owner_only(
        &dir.join("custody.pub"),
        hex::encode(public).as_bytes(),
    )?;
    let mut state = hash_market::server::AppState::new(
        None::<hash_market::VoteExtensionHandler>,
        "headstash-fm-1".into(),
        dir,
        true,
    )?;
    if let Ok(op) = std::env::var("HEADSTASH_OPERATOR") {
        if !op.starts_with("terp1") || op.chars().any(|ch| ch.is_whitespace()) {
            anyhow::bail!("operator");
        }
        state.set_tree_operator(op);
    }
    state.headstash.write().await.install_signer(signer);
    let app = hash_market::server::router(std::sync::Arc::new(state));
    let listener = tokio::net::TcpListener::bind(&bind).await?;
    let local = listener.local_addr()?;
    hash_market::tree_operator::require_loopback_listener(local).map_err(anyhow::Error::msg)?;
    println!("lane_up {bind}");
    axum::serve(listener, app).await?;
    Ok(())
}

fn load_signer() -> anyhow::Result<SigningKey> {
    let path = std::env::var("HEADSTASH_CUSTODY_SEED")?;
    let bytes = std::fs::read(&path)?;
    if bytes.len() != 32 {
        anyhow::bail!("custody seed is {} bytes", bytes.len());
    }
    let mut raw = [0u8; 32];
    raw.copy_from_slice(&bytes);
    Ok(SigningKey::from_bytes(&raw))
}

fn denom_bytes() -> [u8; 32] {
    let mut out = [0u8; 32];
    let hex = "1d56a5879c78333d517942a36e7ea0932e7ffa1a1027e439279e8517580ea706";
    for i in 0..32 {
        out[i] = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).unwrap();
    }
    out
}

fn base64_vk(hash: &[u8; 32]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(hash)
}
