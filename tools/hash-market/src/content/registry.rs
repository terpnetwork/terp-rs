//! Dual-index content registry: BUD sha256 (primary) + optional IPFS CID.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::RwLock;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ContentOrigin {
    /// `bud` | `s3` | `ipfs` | `ingest`
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bucket: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ContentUrls {
    pub bud: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ipfs: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub s3: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ContentRecord {
    /// BUD primary identity — hex sha256 of raw bytes
    pub sha256: String,
    pub size: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ipfs_cid: Option<String>,
    #[serde(default)]
    pub origins: Vec<ContentOrigin>,
    #[serde(default)]
    pub labels: Vec<String>,
    pub updated_at: String,
}

impl ContentRecord {
    pub fn urls(&self, bud_base: &str, ipfs_gateway: &str) -> ContentUrls {
        let base = bud_base.trim_end_matches('/');
        let bud = format!("{base}/blobs/{}", self.sha256);
        let ipfs = self.ipfs_cid.as_ref().map(|cid| {
            let gw = ipfs_gateway.trim();
            if gw.starts_with("http") {
                format!("{}/{cid}", gw.trim_end_matches('/'))
            } else {
                format!("{gw}{cid}")
            }
        });
        let s3 = self.origins.iter().find_map(|o| {
            if o.kind == "s3" {
                match (&o.bucket, &o.key) {
                    (Some(b), Some(k)) => Some(format!("s3://{b}/{k}")),
                    _ => None,
                }
            } else {
                None
            }
        });
        ContentUrls { bud, ipfs, s3 }
    }
}

/// File-backed registry: one JSON file per sha256 + optional cid index.
pub struct ContentRegistry {
    dir: PathBuf,
    /// cid → sha256
    cid_index: RwLock<HashMap<String, String>>,
}

impl ContentRegistry {
    pub fn open(dir: impl AsRef<Path>) -> anyhow::Result<Self> {
        let dir = dir.as_ref().to_path_buf();
        fs::create_dir_all(dir.join("by-sha256"))?;
        let reg = Self {
            dir,
            cid_index: RwLock::new(HashMap::new()),
        };
        reg.rebuild_cid_index()?;
        Ok(reg)
    }

    fn path_for(&self, sha256: &str) -> PathBuf {
        self.dir.join("by-sha256").join(format!("{sha256}.json"))
    }

    fn rebuild_cid_index(&self) -> anyhow::Result<()> {
        let mut idx = self.cid_index.write().unwrap();
        idx.clear();
        let root = self.dir.join("by-sha256");
        if !root.exists() {
            return Ok(());
        }
        for ent in fs::read_dir(root)? {
            let ent = ent?;
            if ent.path().extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            if let Ok(bytes) = fs::read(ent.path()) {
                if let Ok(rec) = serde_json::from_slice::<ContentRecord>(&bytes) {
                    if let Some(cid) = &rec.ipfs_cid {
                        idx.insert(cid.clone(), rec.sha256.clone());
                    }
                }
            }
        }
        Ok(())
    }

    pub fn get(&self, sha256: &str) -> anyhow::Result<Option<ContentRecord>> {
        let p = self.path_for(sha256);
        if !p.exists() {
            return Ok(None);
        }
        let bytes = fs::read(p)?;
        Ok(Some(serde_json::from_slice(&bytes)?))
    }

    pub fn get_by_cid(&self, cid: &str) -> anyhow::Result<Option<ContentRecord>> {
        let sha = {
            let idx = self.cid_index.read().unwrap();
            idx.get(cid).cloned()
        };
        match sha {
            Some(s) => self.get(&s),
            None => Ok(None),
        }
    }

    /// Insert or merge origins/labels/cid.
    pub fn upsert(&self, mut incoming: ContentRecord) -> anyhow::Result<ContentRecord> {
        incoming.sha256 = incoming.sha256.to_lowercase();
        if let Some(mut existing) = self.get(&incoming.sha256)? {
            // merge origins
            for o in incoming.origins.drain(..) {
                if !existing
                    .origins
                    .iter()
                    .any(|e| e.kind == o.kind && e.bucket == o.bucket && e.key == o.key)
                {
                    existing.origins.push(o);
                }
            }
            for l in incoming.labels.drain(..) {
                if !existing.labels.contains(&l) {
                    existing.labels.push(l);
                }
            }
            if incoming.ipfs_cid.is_some() {
                existing.ipfs_cid = incoming.ipfs_cid;
            }
            if incoming.content_type.is_some() {
                existing.content_type = incoming.content_type;
            }
            if incoming.size > 0 {
                existing.size = incoming.size;
            }
            existing.updated_at = incoming.updated_at;
            self.write(&existing)?;
            return Ok(existing);
        }
        self.write(&incoming)?;
        Ok(incoming)
    }

    fn write(&self, rec: &ContentRecord) -> anyhow::Result<()> {
        let p = self.path_for(&rec.sha256);
        let tmp = p.with_extension("tmp");
        fs::write(&tmp, serde_json::to_vec_pretty(rec)?)?;
        fs::rename(&tmp, &p)?;
        if let Some(cid) = &rec.ipfs_cid {
            self.cid_index
                .write()
                .unwrap()
                .insert(cid.clone(), rec.sha256.clone());
        }
        Ok(())
    }

    pub fn list(&self) -> anyhow::Result<Vec<ContentRecord>> {
        let mut out = Vec::new();
        let root = self.dir.join("by-sha256");
        if !root.exists() {
            return Ok(out);
        }
        for ent in fs::read_dir(root)? {
            let ent = ent?;
            if ent.path().extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            if let Ok(bytes) = fs::read(ent.path()) {
                if let Ok(rec) = serde_json::from_slice::<ContentRecord>(&bytes) {
                    out.push(rec);
                }
            }
        }
        out.sort_by(|a, b| a.sha256.cmp(&b.sha256));
        Ok(out)
    }
}
