//! Paths and tunables. Everything mutable lives under one data directory
//! (`%LOCALAPPDATA%\ChordKu\data` on Windows) so the binary itself can sit anywhere,
//! including a read-only location.

use std::net::{IpAddr, Ipv4Addr};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

pub const DEFAULT_PORT: u16 = 7464;

#[derive(Clone, Debug)]
pub struct Config {
    pub data_dir: PathBuf,
    pub port: u16,
    /// Bind address. Loopback unless LAN mode is enabled.
    pub bind: IpAddr,
    /// Longest recording accepted, in seconds.
    pub max_duration_secs: u32,
    /// Largest upload accepted, in bytes.
    pub max_upload_bytes: u64,
    /// ONNX Runtime intra-op threads (0 = automatic).
    pub threads: usize,
    /// A separation is refused when less physical memory than this is available (bytes).
    pub min_free_ram: u64,
    /// Extra directories searched for model files before `data_dir/models`.
    pub model_search: Vec<PathBuf>,
}

impl Config {
    pub fn from_env() -> Result<Self> {
        let data_dir = match std::env::var_os("CHORDKU_DATA") {
            Some(p) => PathBuf::from(p),
            None => directories::ProjectDirs::from("", "", "ChordKu")
                .context("no home directory")?
                .data_local_dir()
                .to_path_buf(),
        };
        let mut model_search = Vec::new();
        if let Some(p) = std::env::var_os("CHORDKU_MODELS") {
            model_search.push(PathBuf::from(p));
        }
        if let Ok(exe) = std::env::current_exe() {
            if let Some(dir) = exe.parent() {
                model_search.push(dir.join("models"));
            }
        }
        if let Ok(cwd) = std::env::current_dir() {
            model_search.push(cwd.join("models"));
        }
        let env_num = |k: &str| std::env::var(k).ok().and_then(|v| v.parse::<u64>().ok());
        let cfg = Self {
            data_dir,
            port: env_num("CHORDKU_PORT").map(|p| p as u16).unwrap_or(DEFAULT_PORT),
            bind: IpAddr::V4(Ipv4Addr::LOCALHOST),
            max_duration_secs: env_num("CHORDKU_MAX_SECONDS").map(|v| v as u32).unwrap_or(20 * 60),
            max_upload_bytes: env_num("CHORDKU_MAX_UPLOAD_MB").unwrap_or(300) * 1024 * 1024,
            threads: env_num("CHORDKU_THREADS").unwrap_or(0) as usize,
            min_free_ram: env_num("CHORDKU_MIN_FREE_MB").unwrap_or(1536) * 1024 * 1024,
            model_search,
        };
        for dir in [cfg.bin_dir(), cfg.models_dir(), cfg.media_dir(), cfg.tmp_dir()] {
            std::fs::create_dir_all(&dir).with_context(|| format!("create {}", dir.display()))?;
        }
        Ok(cfg)
    }

    pub fn db_path(&self) -> PathBuf {
        self.data_dir.join("chordku.sqlite")
    }
    pub fn bin_dir(&self) -> PathBuf {
        self.data_dir.join("bin")
    }
    /// Where downloaded models are stored.
    pub fn models_dir(&self) -> PathBuf {
        self.data_dir.join("models")
    }
    pub fn media_dir(&self) -> PathBuf {
        self.data_dir.join("media")
    }
    pub fn tmp_dir(&self) -> PathBuf {
        self.data_dir.join("tmp")
    }
    pub fn song_dir(&self, song_id: &str) -> PathBuf {
        self.media_dir().join(song_id)
    }

    /// First location that holds `file`, searching developer / portable layouts first.
    pub fn find_model(&self, file: &str) -> Option<PathBuf> {
        self.model_search
            .iter()
            .map(PathBuf::as_path)
            .chain(std::iter::once(self.models_dir().as_path()))
            .map(|d: &Path| d.join(file))
            .find(|p| p.is_file())
    }
}
