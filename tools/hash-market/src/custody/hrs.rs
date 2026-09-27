//! Double-sign floor (height/round/step). Persist-first, fail closed.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct HrsState {
    pub height: i64,
    pub round: i32,
    pub step: i8,
    #[serde(default)]
    pub sig_hex: String,
}

pub struct HrsLock {
    path: PathBuf,
    state: HrsState,
}

impl HrsLock {
    pub fn load(path: &Path) -> Result<Self> {
        if path.exists() {
            let raw = fs::read_to_string(path).context("hrs state")?;
            let state: HrsState = serde_json::from_str(&raw).context("hrs json")?;
            return Ok(Self {
                path: path.to_path_buf(),
                state,
            });
        }
        Ok(Self {
            path: path.to_path_buf(),
            state: HrsState::default(),
        })
    }

    /// Ok(Some(cached)) if same HRS already signed. Err if regression. Ok(None) if new.
    pub fn check(&self, height: i64, round: i32, step: i8) -> Result<Option<Vec<u8>>> {
        let s = &self.state;
        if height < s.height
            || (height == s.height && round < s.round)
            || (height == s.height && round == s.round && step < s.step)
        {
            anyhow::bail!(
                "double-sign: refuse H/R/S {}/{}/{} after {}/{}/{}",
                height,
                round,
                step,
                s.height,
                s.round,
                s.step
            );
        }
        if height == s.height && round == s.round && step == s.step && !s.sig_hex.is_empty() {
            return Ok(Some(hex::decode(&s.sig_hex)?));
        }
        Ok(None)
    }

    pub fn commit(&mut self, height: i64, round: i32, step: i8, sig: &[u8]) -> Result<()> {
        self.state = HrsState {
            height,
            round,
            step,
            sig_hex: hex::encode(sig),
        };
        if let Some(dir) = self.path.parent() {
            fs::create_dir_all(dir)?;
        }
        let tmp = self.path.with_extension("json.tmp");
        fs::write(&tmp, serde_json::to_vec_pretty(&self.state)?)?;
        fs::rename(tmp, &self.path)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_hrs_returns_cached() {
        let dir = tempfile_dir();
        let p = dir.join("hrs.json");
        let mut l = HrsLock::load(&p).unwrap();
        assert!(l.check(1, 0, 2).unwrap().is_none());
        l.commit(1, 0, 2, &[1, 2, 3]).unwrap();
        let again = HrsLock::load(&p).unwrap();
        assert_eq!(again.check(1, 0, 2).unwrap().unwrap(), vec![1, 2, 3]);
        assert!(again.check(1, 0, 1).is_err());
    }

    fn tempfile_dir() -> PathBuf {
        let p = std::env::temp_dir().join(format!("hrs-{}", std::process::id()));
        let _ = fs::create_dir_all(&p);
        p
    }
}
