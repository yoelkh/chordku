//! Model files: what is needed, where it comes from, and verified downloads.
//!
//! Nothing here is redistributed by ChordKu. Beat This! (MIT) and htdemucs (weights offered
//! by their authors for research use only) are fetched from their original publishers on
//! the user's request; the BTC export (MIT) is produced locally by tools/export.

use std::path::PathBuf;

use anyhow::{anyhow, bail, Context, Result};
use futures_util::StreamExt;
use serde::Serialize;
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;

use crate::config::Config;

#[derive(Clone, Copy, Debug, Serialize)]
pub struct ModelFile {
    pub file: &'static str,
    pub bytes: u64,
    pub sha256: &'static str,
    /// `None`: produced locally by `tools/export/export_btc.py`.
    pub url: Option<&'static str>,
}

#[derive(Clone, Copy, Debug, Serialize)]
pub struct ModelGroup {
    pub id: &'static str,
    pub title: &'static str,
    pub license: &'static str,
    pub source: &'static str,
    /// Needed before any song can be analysed.
    pub required: bool,
    pub files: &'static [ModelFile],
}

pub const GROUPS: &[ModelGroup] = &[
    ModelGroup {
        id: "chords",
        title: "Pengenal chord BTC (170 kelas)",
        license: "MIT",
        source: "https://github.com/jayg996/BTC-ISMIR19",
        required: true,
        files: &[
            ModelFile {
                file: "btc.onnx",
                bytes: 12_940_595,
                sha256: "7444c19cd1bd9cb50af656fafc939f1894f63f3bff41d5942c7d1d4fab2b7ed5",
                url: None,
            },
            ModelFile { file: "btc.json", bytes: 0, sha256: "", url: None },
        ],
    },
    ModelGroup {
        id: "beats",
        title: "Pelacak beat Beat This! (small)",
        license: "MIT",
        source: "https://github.com/CPJKU/beat_this",
        required: true,
        files: &[
            ModelFile {
                file: "mel_spectrogram.onnx",
                bytes: 270_742,
                sha256: "fdd59e65c515331308e4c8841edf99972deca646bdf6197744c2a5b7755e3de9",
                url: Some("https://github.com/danigb/beat-this-rs/raw/main/models/mel_spectrogram.onnx"),
            },
            ModelFile {
                file: "beat_this_small.onnx",
                bytes: 10_555_592,
                sha256: "a5f8d39d989f31859454ba27afe61c5317ca95e4d9373e6853e5361b8937172f",
                url: Some("https://github.com/danigb/beat-this-rs/raw/main/models/beat_this_small.onnx"),
            },
        ],
    },
    ModelGroup {
        id: "stems",
        title: "Pemisah stem htdemucs (vokal, drum, bass, lainnya)",
        license: "Kode MIT; bobot hanya untuk keperluan riset/pribadi, tidak boleh disebarkan ulang",
        source: "https://huggingface.co/Intel/demucs-openvino",
        required: false,
        files: &[
            ModelFile {
                file: "htdemucs_fwd.onnx",
                bytes: 2_385_507,
                sha256: "555f511fb653348a1c9495ab6b6091767d1920798f3fba68629a1fe5b135a45f",
                url: Some("https://huggingface.co/Intel/demucs-openvino/resolve/main/htdemucs_v4/htdemucs_fwd.onnx"),
            },
            ModelFile {
                file: "htdemucs_fwd.onnx.data",
                bytes: 168_361_984,
                sha256: "23a7c68669c041363caaef94641e961dc8c4e16b9891cb87f58f0e199a5e8820",
                url: Some("https://huggingface.co/Intel/demucs-openvino/resolve/main/htdemucs_v4/htdemucs_fwd.onnx.data"),
            },
        ],
    },
];

#[derive(Clone, Debug, Serialize)]
pub struct GroupStatus {
    #[serde(flatten)]
    pub group: ModelGroup,
    pub installed: bool,
    pub missing: Vec<&'static str>,
    pub downloadable: bool,
    pub download_bytes: u64,
}

pub fn group(id: &str) -> Option<&'static ModelGroup> {
    GROUPS.iter().find(|g| g.id == id)
}

pub fn status(cfg: &Config) -> Vec<GroupStatus> {
    GROUPS
        .iter()
        .map(|g| {
            let missing: Vec<&'static str> = g.files.iter().filter(|f| cfg.find_model(f.file).is_none()).map(|f| f.file).collect();
            GroupStatus {
                group: *g,
                installed: missing.is_empty(),
                downloadable: g.files.iter().all(|f| f.url.is_some()),
                download_bytes: g.files.iter().filter(|f| missing.contains(&f.file)).map(|f| f.bytes).sum(),
                missing,
            }
        })
        .collect()
}

/// Directory holding every file of the group, if it is completely installed.
/// (All files of a group must sit together: ONNX external data is resolved by relative path.)
pub fn group_dir(cfg: &Config, id: &str) -> Option<PathBuf> {
    let g = group(id)?;
    let first = cfg.find_model(g.files.first()?.file)?;
    let dir = first.parent()?.to_path_buf();
    g.files.iter().all(|f| dir.join(f.file).is_file()).then_some(dir)
}

/// Download the missing files of a group into the data directory, verifying size and hash.
pub async fn download(cfg: &Config, id: &str, mut progress: impl FnMut(f64)) -> Result<()> {
    let g = group(id).ok_or_else(|| anyhow!("model tidak dikenal: {id}"))?;
    let dir = cfg.models_dir();
    let todo: Vec<&ModelFile> = g.files.iter().filter(|f| !dir.join(f.file).is_file()).collect();
    if todo.iter().any(|f| f.url.is_none()) {
        bail!("model ini dibuat lokal: jalankan tools\\setup.ps1 (export BTC ke ONNX)");
    }
    let total: u64 = todo.iter().map(|f| f.bytes).sum::<u64>().max(1);
    let mut done = 0u64;
    let client = reqwest::Client::builder().user_agent(concat!("ChordKu/", env!("CARGO_PKG_VERSION"))).build()?;
    for f in todo {
        let part = dir.join(format!("{}.part", f.file));
        let mut out = tokio::fs::File::create(&part).await.with_context(|| format!("buat {}", part.display()))?;
        let mut hasher = Sha256::new();
        let mut got = 0u64;
        let resp = client.get(f.url.expect("checked above")).send().await?.error_for_status()?;
        let mut stream = resp.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.context("unduhan terputus")?;
            got += chunk.len() as u64;
            if got > f.bytes {
                drop(out);
                let _ = tokio::fs::remove_file(&part).await;
                bail!("{} lebih besar dari yang diharapkan", f.file);
            }
            hasher.update(&chunk);
            out.write_all(&chunk).await?;
            progress((done + got) as f64 / total as f64);
        }
        out.flush().await?;
        drop(out);
        if got != f.bytes || format!("{:x}", hasher.finalize()) != f.sha256 {
            let _ = tokio::fs::remove_file(&part).await;
            bail!("{} rusak atau berubah di sumbernya (checksum tidak cocok)", f.file);
        }
        tokio::fs::rename(&part, dir.join(f.file)).await?;
        done += f.bytes;
    }
    progress(1.0);
    Ok(())
}
