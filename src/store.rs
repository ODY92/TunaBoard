use crate::catalog::Sound;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::{env, path::PathBuf};

#[derive(Default, Serialize, Deserialize)]
pub struct Library {
    #[serde(default)]
    pub sounds: Vec<Sound>,
}

pub fn data_dir() -> PathBuf {
    base("XDG_DATA_HOME", ".local/share").join("tuna-board")
}
pub fn cache_dir() -> PathBuf {
    base("XDG_CACHE_HOME", ".cache").join("tuna-board")
}
fn base(key: &str, fallback: &str) -> PathBuf {
    env::var_os(key)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env::var_os("HOME").unwrap_or_default()).join(fallback))
}
impl Library {
    pub fn load() -> Result<Self> {
        let p = data_dir().join("library.json");
        if !p.exists() {
            return Ok(Self::default());
        }
        serde_json::from_slice(&std::fs::read(p)?)
            .context("Bibliothèque illisible : le fichier existant est conservé")
    }
    pub fn save(&self) -> Result<()> {
        let d = data_dir();
        std::fs::create_dir_all(&d)?;
        let tmp = d.join("library.json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(self)?)?;
        std::fs::rename(tmp, d.join("library.json"))?;
        Ok(())
    }
}
