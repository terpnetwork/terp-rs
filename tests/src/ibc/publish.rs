//! Atomic publish helpers: stage → validate caller-side → promote into out/.

use sha2::{Digest, Sha256};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Default public dir: `<scripts CARGO_MANIFEST_DIR>/../public` (repo-root public/).
pub fn default_public_dir(manifest_dir: &Path) -> PathBuf {
    manifest_dir
        .join("..")
        .join("public")
        .canonicalize()
        .unwrap_or_else(|_| manifest_dir.join("..").join("public"))
}

/// Resolve CLI `--out` / legacy `--public-dir`, defaulting to repo-root public/.
pub fn resolve_out_dir(cli_out: Option<&Path>, manifest_dir: &Path) -> PathBuf {
    match cli_out {
        Some(p) if p.as_os_str().is_empty() => default_public_dir(manifest_dir),
        Some(p) => {
            if p.is_absolute() {
                p.to_path_buf()
            } else {
                std::env::current_dir()
                    .map(|c| c.join(p))
                    .unwrap_or_else(|_| p.to_path_buf())
            }
        }
        None => default_public_dir(manifest_dir),
    }
}

/// SHA-256 hex of file bytes.
pub fn file_sha256(path: &Path) -> io::Result<String> {
    let bytes = fs::read(path)?;
    let mut h = Sha256::new();
    h.update(&bytes);
    Ok(hex::encode(h.finalize()))
}

/// Stages files under `out/_staging/<id>/` then promotes into `out/`.
pub struct AtomicPublisher {
    pub out: PathBuf,
    pub staging: PathBuf,
}

impl AtomicPublisher {
    pub fn create(out: &Path) -> io::Result<Self> {
        let id = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let staging = out.join("_staging").join(format!("run-{id}"));
        fs::create_dir_all(&staging)?;
        Ok(Self {
            out: out.to_path_buf(),
            staging,
        })
    }

    /// Write relative path under staging (creates parents).
    pub fn write_rel(&self, rel: &str, contents: impl AsRef<[u8]>) -> io::Result<PathBuf> {
        let path = self.staging.join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&path, contents)?;
        Ok(path)
    }

    /// Promote every file under staging into out/, preserving relative paths.
    /// On success, removes this staging dir. On failure leaves staging intact.
    pub fn promote(self) -> io::Result<Vec<PathBuf>> {
        let mut promoted = Vec::new();
        self.promote_walk(&self.staging, &self.out, &mut promoted)?;
        // Best-effort cleanup of empty staging tree
        let _ = fs::remove_dir_all(&self.staging);
        let staging_root = self.out.join("_staging");
        if staging_root.is_dir() {
            if let Ok(mut rd) = fs::read_dir(&staging_root) {
                if rd.next().is_none() {
                    let _ = fs::remove_dir(&staging_root);
                }
            }
        }
        Ok(promoted)
    }

    fn promote_walk(&self, from: &Path, to_base: &Path, out: &mut Vec<PathBuf>) -> io::Result<()> {
        for ent in fs::read_dir(from)? {
            let ent = ent?;
            let src = ent.path();
            let name = ent.file_name();
            let dest = to_base.join(&name);
            if src.is_dir() {
                fs::create_dir_all(&dest)?;
                self.promote_walk(&src, &dest, out)?;
            } else {
                if let Some(parent) = dest.parent() {
                    fs::create_dir_all(parent)?;
                }
                // Atomic-ish replace: write temp then rename
                let tmp = dest.with_extension(format!(
                    "{}.tmp",
                    dest.extension().and_then(|e| e.to_str()).unwrap_or("bin")
                ));
                fs::copy(&src, &tmp)?;
                fs::rename(&tmp, &dest)?;
                out.push(dest);
            }
        }
        Ok(())
    }

    /// Discard staging without promoting (e.g. after invariant failure).
    pub fn discard(self) -> io::Result<()> {
        if self.staging.exists() {
            fs::remove_dir_all(&self.staging)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn atomic_promote_writes_final_only_after_promote() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("public");
        fs::create_dir_all(&out).unwrap();
        let pubr = AtomicPublisher::create(&out).unwrap();
        pubr.write_rel("ibc_lookup_table.json", b"{\"ok\":true}")
            .unwrap();
        assert!(!out.join("ibc_lookup_table.json").exists());
        let paths = pubr.promote().unwrap();
        assert_eq!(paths.len(), 1);
        assert!(out.join("ibc_lookup_table.json").exists());
        assert_eq!(
            fs::read_to_string(out.join("ibc_lookup_table.json")).unwrap(),
            "{\"ok\":true}"
        );
    }

    #[test]
    fn discard_leaves_out_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("public");
        fs::create_dir_all(&out).unwrap();
        fs::write(out.join("keep.json"), b"orig").unwrap();
        let pubr = AtomicPublisher::create(&out).unwrap();
        pubr.write_rel("keep.json", b"new").unwrap();
        pubr.discard().unwrap();
        assert_eq!(fs::read_to_string(out.join("keep.json")).unwrap(), "orig");
    }
}
