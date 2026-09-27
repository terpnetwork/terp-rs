//! Ingest `artifacts/community-notes/notes.json` as public leaf blobs + merkle paths.
//!
//! Stored under `{data_dir}/pir/{hs_id}/` — not `notes/{hs_id}/{addr}.json`.
//! `/notes/{hs_id}/pir` XORs these blobs. It does not read a single address file.
//! NIP-BUD / blossom and encrypted GET-by-addr stay on their existing routes.

use std::path::{Path, PathBuf};

use anyhow::Context;
use serde::{Deserialize, Serialize};

use crate::store::{validate_id, HeadstashStore};

use super::poseidon::{auth_tree, fp_from_hex, fp_to_hex, path_root};

/// Public record returned (after client-side XOR decode) by `POST /notes/{hs_id}/pir`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PublicLeafBlob {
    pub leaf: String,
    pub path: Vec<String>,
    pub fdi: u64,
    pub nd: String,
    pub v: u64,
    pub recp: String,
    pub epk_x: String,
    pub epk_y: String,
    pub addr: String,
    pub community: String,
    pub elig_pk_compressed: String,
}

/// Public index for selector setup. Not a per-address fetch.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PirIndex {
    pub keys: Vec<String>,
    pub addrs: Vec<String>,
    pub root: String,
    pub count: u64,
    pub domain: String,
}

#[derive(Debug, Clone)]
pub struct PirIngestReport {
    pub hs_id: String,
    pub count: usize,
    pub root: String,
    pub skipped_existing: bool,
}

#[derive(Debug, Deserialize)]
struct NotesFile {
    notes: Vec<GenesisNote>,
}

#[derive(Debug, Deserialize)]
struct GenesisNote {
    addr: String,
    community: String,
    elig_pk_compressed: String,
    epk_x: String,
    epk_y: String,
    nd: String,
    v: u64,
    fdi: u64,
    recp: String,
    leaf: String,
}

#[derive(Debug, Deserialize)]
struct MerkleFile {
    root: String,
    count: u64,
    distro_hash_domain: String,
}

fn pir_root(store: &HeadstashStore, hs_id: &str) -> PathBuf {
    store.base_dir().join("pir").join(hs_id)
}

fn norm_hex(s: &str) -> String {
    s.trim()
        .trim_start_matches("0x")
        .trim_start_matches("0X")
        .to_ascii_lowercase()
}

fn fdi_key(fdi: u64) -> String {
    format!("{fdi:08}")
}

fn write_bytes_atomic(path: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

/// Load the public index (`meta.json`) if this `hs_id` was ingested.
pub fn load_pir_index(store: &HeadstashStore, hs_id: &str) -> anyhow::Result<Option<PirIndex>> {
    validate_id(hs_id)?;
    let path = pir_root(store, hs_id).join("meta.json");
    if !path.exists() {
        return Ok(None);
    }
    let raw = std::fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
    Ok(Some(serde_json::from_str(&raw)?))
}

/// All public leaf blobs in index order, NUL-padded to equal length for XOR PIR.
/// Reads every blob. Does not open `notes/{hs_id}/{addr}.json`.
pub fn load_pir_blobs(
    store: &HeadstashStore,
    hs_id: &str,
) -> anyhow::Result<Option<(Vec<String>, Vec<Vec<u8>>)>> {
    let Some(index) = load_pir_index(store, hs_id)? else {
        return Ok(None);
    };
    let dir = pir_root(store, hs_id).join("blobs");
    let mut blobs = Vec::with_capacity(index.keys.len());
    for key in &index.keys {
        validate_id(key)?;
        let path = dir.join(format!("{key}.json"));
        let data = std::fs::read(&path).with_context(|| format!("read pir blob {}", path.display()))?;
        blobs.push(data);
    }
    let max_len = blobs.iter().map(|b| b.len()).max().unwrap_or(0);
    for blob in &mut blobs {
        blob.resize(max_len, 0);
    }
    Ok(Some((index.keys, blobs)))
}

/// Ingest genesis notes as public leaf blobs plus a Poseidon-v1 merkle path.
///
/// Rebuilds only when `merkle_output.json` root/count/domain differ from `meta.json`.
/// Does not write encrypted note envelopes and does not touch blossom blob storage.
pub fn ingest_community_notes(
    store: &HeadstashStore,
    hs_id: &str,
    notes_json: &Path,
    merkle_json: &Path,
) -> anyhow::Result<PirIngestReport> {
    validate_id(hs_id)?;
    let merkle_raw = std::fs::read_to_string(merkle_json)
        .with_context(|| format!("read merkle {}", merkle_json.display()))?;
    let merkle: MerkleFile = serde_json::from_str(&merkle_raw)?;
    anyhow::ensure!(
        merkle.distro_hash_domain == "poseidon-v1",
        "refusing non-poseidon-v1 tree ({})",
        merkle.distro_hash_domain
    );
    anyhow::ensure!(merkle.count > 0, "merkle count is 0");

    if let Some(existing) = load_pir_index(store, hs_id)? {
        let blob_dir = pir_root(store, hs_id).join("blobs");
        let on_disk = if blob_dir.exists() {
            std::fs::read_dir(&blob_dir)?.count()
        } else {
            0
        };
        if existing.count == merkle.count
            && existing.domain == merkle.distro_hash_domain
            && norm_hex(&existing.root) == norm_hex(&merkle.root)
            && on_disk == merkle.count as usize
            && existing.keys.len() == merkle.count as usize
        {
            return Ok(PirIngestReport {
                hs_id: hs_id.to_string(),
                count: existing.keys.len(),
                root: existing.root,
                skipped_existing: true,
            });
        }
    }

    let notes_raw = std::fs::read_to_string(notes_json)
        .with_context(|| format!("read notes {}", notes_json.display()))?;
    let mut file: NotesFile = serde_json::from_str(&notes_raw)?;
    anyhow::ensure!(
        file.notes.len() as u64 == merkle.count,
        "notes.len() {} != merkle count {}",
        file.notes.len(),
        merkle.count
    );
    file.notes.sort_by_key(|n| n.fdi);
    for (i, note) in file.notes.iter().enumerate() {
        anyhow::ensure!(
            note.fdi == i as u64,
            "fdi must be contiguous from 0, found {} at position {i}",
            note.fdi
        );
    }

    let mut leaves = Vec::with_capacity(file.notes.len());
    for note in &file.notes {
        leaves.push(fp_from_hex(&note.leaf).with_context(|| format!("leaf fdi {}", note.fdi))?);
    }
    let tree = auth_tree(&leaves);
    let root_hex = fp_to_hex(tree.root);
    anyhow::ensure!(
        norm_hex(&root_hex) == norm_hex(&merkle.root),
        "computed poseidon root {root_hex} != merkle file {}",
        merkle.root
    );

    let mut records = Vec::with_capacity(file.notes.len());
    let mut keys = Vec::with_capacity(file.notes.len());
    let mut addrs = Vec::with_capacity(file.notes.len());
    for (i, note) in file.notes.iter().enumerate() {
        let path_fps = &tree.paths[i];
        let got = path_root(leaves[i], i as u32, path_fps);
        anyhow::ensure!(got == tree.root, "merkle path failed for fdi {i}");
        let key = fdi_key(note.fdi);
        validate_id(&key)?;
        keys.push(key);
        addrs.push(note.addr.clone());
        records.push(PublicLeafBlob {
            leaf: fp_to_hex(leaves[i]),
            path: path_fps.iter().copied().map(fp_to_hex).collect(),
            fdi: note.fdi,
            nd: note.nd.clone(),
            v: note.v,
            recp: note.recp.clone(),
            epk_x: note.epk_x.clone(),
            epk_y: note.epk_y.clone(),
            addr: note.addr.clone(),
            community: note.community.clone(),
            elig_pk_compressed: note.elig_pk_compressed.clone(),
        });
    }

    let root_dir = pir_root(store, hs_id);
    let blob_dir = root_dir.join("blobs");
    if blob_dir.exists() {
        std::fs::remove_dir_all(&blob_dir)?;
    }
    std::fs::create_dir_all(&blob_dir)?;
    let meta_path = root_dir.join("meta.json");
    if meta_path.exists() {
        std::fs::remove_file(&meta_path)?;
    }
    for (key, rec) in keys.iter().zip(records.iter()) {
        let bytes = serde_json::to_vec(rec)?;
        write_bytes_atomic(&blob_dir.join(format!("{key}.json")), &bytes)?;
    }
    let index = PirIndex {
        keys,
        addrs,
        root: root_hex.clone(),
        count: records.len() as u64,
        domain: merkle.distro_hash_domain,
    };
    write_bytes_atomic(&meta_path, &serde_json::to_vec(&index)?)?;

    Ok(PirIngestReport {
        hs_id: hs_id.to_string(),
        count: records.len(),
        root: root_hex,
        skipped_existing: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::{router, AppState};
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use pasta_curves::pallas;
    use std::sync::Arc;
    use std::time::{SystemTime, UNIX_EPOCH};
    use tower::ServiceExt;

    fn tmp_dir(label: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("hash-market-pir-{label}-{nanos}"));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_fixture(dir: &Path, n: u64) -> (PathBuf, PathBuf, String) {
        let mut notes = Vec::new();
        let mut leaves = Vec::new();
        for i in 0..n {
            let leaf = pallas::Base::from(i + 1);
            leaves.push(leaf);
            notes.push(serde_json::json!({
                "addr": format!("0x{:040x}", i + 1),
                "community": "buddah-bears",
                "elig_pk_compressed": "0x02aa",
                "epk_x": "0x11",
                "epk_y": "0x22",
                "nd": "0x33",
                "v": 1,
                "fdi": i,
                "recp": "0x44",
                "leaf": fp_to_hex(leaf),
                "esk_hint": "must-not-appear-in-blob",
                "token": "buddah-bears"
            }));
        }
        let tree = auth_tree(&leaves);
        let root = fp_to_hex(tree.root);
        let notes_path = dir.join("notes.json");
        let merkle_path = dir.join("merkle_output.json");
        std::fs::write(
            &notes_path,
            serde_json::to_vec(&serde_json::json!({ "notes": notes })).unwrap(),
        )
        .unwrap();
        std::fs::write(
            &merkle_path,
            serde_json::to_vec(&serde_json::json!({
                "root": root,
                "count": n,
                "distro_hash_domain": "poseidon-v1",
                "holders_keyed": n,
                "threads": 1
            }))
            .unwrap(),
        )
        .unwrap();
        (notes_path, merkle_path, root)
    }

    #[test]
    fn ingest_writes_public_blobs_not_addr_notes() {
        let dir = tmp_dir("ingest");
        let (notes_path, merkle_path, root) = write_fixture(&dir, 3);
        let store = HeadstashStore::new(&dir).unwrap();
        let report = ingest_community_notes(&store, "community-notes", &notes_path, &merkle_path)
            .unwrap();
        assert!(!report.skipped_existing);
        assert_eq!(report.count, 3);
        assert_eq!(norm_hex(&report.root), norm_hex(&root));
        assert!(!dir.join("notes").join("community-notes").exists());
        let blob = dir
            .join("pir")
            .join("community-notes")
            .join("blobs")
            .join("00000001.json");
        let raw = std::fs::read(&blob).unwrap();
        let parsed: PublicLeafBlob = serde_json::from_slice(&raw).unwrap();
        assert_eq!(parsed.fdi, 1);
        assert!(!parsed.path.is_empty());
        assert!(!raw.windows(8).any(|w| w == b"esk_hint"));
        let again =
            ingest_community_notes(&store, "community-notes", &notes_path, &merkle_path).unwrap();
        assert!(again.skipped_existing);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn pir_http_does_not_get_by_addr() {
        let dir = tmp_dir("http");
        let (notes_path, merkle_path, _) = write_fixture(&dir, 2);
        let state = AppState::new_merkle_host("test-chain".into(), dir.clone()).unwrap();
        ingest_community_notes(
            state.headstash_store.as_ref(),
            "community-notes",
            &notes_path,
            &merkle_path,
        )
        .unwrap();
        let addr = format!("0x{:040x}", 2u64);
        assert!(state
            .headstash_store
            .get_note("community-notes", &addr)
            .unwrap()
            .is_none());
        let app = router(Arc::new(state));

        let health = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(health.status(), StatusCode::OK);

        let index = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri("/notes/community-notes/pir-index")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(index.status(), StatusCode::OK);
        let index_bytes = axum::body::to_bytes(index.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let index: PirIndex = serde_json::from_slice(&index_bytes).unwrap();
        assert_eq!(index.keys, vec!["00000000".to_string(), "00000001".to_string()]);
        assert_eq!(index.addrs[1], addr);

        let pir = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/notes/community-notes/pir")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"selector":[0,1]}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(pir.status(), StatusCode::OK);
        let pir_bytes = axum::body::to_bytes(pir.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let pir_json: serde_json::Value = serde_json::from_slice(&pir_bytes).unwrap();
        let raw = hex::decode(pir_json["result"].as_str().unwrap()).unwrap();
        let end = raw.iter().rposition(|b| *b != 0).unwrap() + 1;
        let blob: PublicLeafBlob = serde_json::from_slice(&raw[..end]).unwrap();
        assert_eq!(blob.fdi, 1);
        assert_eq!(blob.addr, addr);
        assert!(!blob.path.is_empty());
        assert!(blob.leaf.starts_with("0x"));

        let by_addr = app
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(format!("/notes/community-notes/{addr}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(by_addr.status(), StatusCode::NOT_FOUND);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Published genesis artifacts. Skips when the checkout has no notes file.
    /// Confirms Poseidon-v1 paths against `merkle_output.json` without writing note envelopes.
    #[test]
    fn ingest_published_community_notes() {
        let art = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../../../crates/headstash/artifacts/community-notes");
        let notes_path = art.join("notes.json");
        let merkle_path = art.join("merkle_output.json");
        if !notes_path.is_file() || !merkle_path.is_file() {
            return;
        }
        let dir = tmp_dir("published");
        let store = HeadstashStore::new(&dir).unwrap();
        let report =
            ingest_community_notes(&store, "community-notes", &notes_path, &merkle_path).unwrap();
        assert!(!report.skipped_existing);
        assert!(report.count > 0);
        let merkle: MerkleFile =
            serde_json::from_str(&std::fs::read_to_string(&merkle_path).unwrap()).unwrap();
        assert_eq!(report.count as u64, merkle.count);
        assert_eq!(norm_hex(&report.root), norm_hex(&merkle.root));
        assert!(!dir.join("notes").exists());
        let index = load_pir_index(&store, "community-notes").unwrap().unwrap();
        assert_eq!(index.keys.len(), report.count);
        assert_eq!(index.addrs.len(), report.count);
        let (_keys, blobs) = load_pir_blobs(&store, "community-notes").unwrap().unwrap();
        assert_eq!(blobs.len(), report.count);
        let raw = &blobs[0];
        let end = raw.iter().rposition(|b| *b != 0).unwrap() + 1;
        let blob: PublicLeafBlob = serde_json::from_slice(&raw[..end]).unwrap();
        assert_eq!(blob.fdi, 0);
        assert!(!blob.path.is_empty() || report.count == 1);
        assert!(!raw.windows(8).any(|w| w == b"esk_hint"));
        let again =
            ingest_community_notes(&store, "community-notes", &notes_path, &merkle_path).unwrap();
        assert!(again.skipped_existing);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
