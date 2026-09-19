//! NVIDIA's CUDA 13 + cuDNN 9 runtime libraries, fetched on request.
//!
//! The CUDA execution provider needs about 1.1 GB of NVIDIA DLLs. They are not bundled:
//! the user downloads them from NVIDIA's official PyPI packages (wheels are zip files), each
//! verified against a pinned SHA-256, and only the DLLs are kept, in `<data>/cuda`.
//! Without them ChordKu simply keeps using DirectML and the CPU.

use std::io::Read;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use futures_util::StreamExt;
use serde::Serialize;
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;

use crate::config::Config;

struct CudaWheel {
    name: &'static str,
    bytes: u64,
    sha256: &'static str,
    url: &'static str,
}

const WHEELS: &[CudaWheel] = &[
    CudaWheel { name: "nvidia-cuda-runtime", bytes: 2778543, sha256: "08dca5e4aba480c2fd5b55075c0fa71b84ef9dcf0521f2d58baa14a803a7311c", url: "https://files.pythonhosted.org/packages/86/00/d5436004268f049214193659ebc36550b5ef3925c3d13b4cc980e13be6f5/nvidia_cuda_runtime-13.4.92-py3-none-win_amd64.whl" },
    CudaWheel { name: "nvidia-cuda-nvrtc", bytes: 46972283, sha256: "6af7ac5372920f6a7a560d0699348560fe44cb52c1d722d6afd3119f8af948c4", url: "https://files.pythonhosted.org/packages/64/5b/aa91896f64444eff1dd7f189f4db05046c7f0ba08c0f531283c1c1aac581/nvidia_cuda_nvrtc-13.4.92-py3-none-win_amd64.whl" },
    CudaWheel { name: "nvidia-curand", bytes: 56731059, sha256: "e0bce83e083ef25976ee74f59e8f067c15149a74538f1be6c2462e286e7c9c68", url: "https://files.pythonhosted.org/packages/e2/d4/f59ab342b82ff05ff27d674acd470ba625d788bf71338f7da6c89669d6b9/nvidia_curand-10.4.4.72-py3-none-win_amd64.whl" },
    CudaWheel { name: "nvidia-cufft", bytes: 160953139, sha256: "4ff7075f2d0b5f69291f70938d37a86ec632cbe5747184c74ba1f50f17accacc", url: "https://files.pythonhosted.org/packages/7b/cc/be7fe31058127336a66c88414a2ecc6beecf680baf85994a43eaf292b202/nvidia_cufft-12.4.0.43-py3-none-win_amd64.whl" },
    CudaWheel { name: "nvidia-cublas", bytes: 423266897, sha256: "8c5494423bb8a46822cb6b0cb95d7fa4be2d7b96a31155dff083839ec8297910", url: "https://files.pythonhosted.org/packages/a3/df/f1246959833e2c437db8be3e5b477f66b87f8817821ed40de6c7561c9a36/nvidia_cublas-13.8.0.4-py3-none-win_amd64.whl" },
    CudaWheel { name: "nvidia-cudnn-cu13", bytes: 419603885, sha256: "8e37a83d7dd7c2663afcae38b1c95c5e9e7c9754f313800da95ca81439fa37f9", url: "https://files.pythonhosted.org/packages/d6/1f/9c5d7ed254d7040192b4bb4eef669ba3ed66a3c4576c50f7d2385b255c8e/nvidia_cudnn_cu13-9.26.0.51-py3-none-win_amd64.whl" },
];

/// Present when the runtime is complete enough for ONNX Runtime's CUDA provider.
const REQUIRED: &[&str] = &["cudart64_13.dll", "cublas64_13.dll", "cublasLt64_13.dll", "cufft64_12.dll", "cudnn64_9.dll"];
/// Shipped in the wheels but never loaded by ONNX Runtime.
const SKIP: &[&str] = &["nvblas64_13.dll", "cufftw64_12.dll"];

#[derive(Clone, Debug, Serialize)]
pub struct CudaStatus {
    /// This build contains the CUDA execution provider.
    pub supported: bool,
    /// An NVIDIA driver is present (nvcuda.dll).
    pub nvidia_gpu: bool,
    pub installed: bool,
    pub dir: String,
    pub download_bytes: u64,
}

pub fn dir(cfg: &Config) -> PathBuf {
    std::env::var_os("CHORDKU_CUDA").map(PathBuf::from).unwrap_or_else(|| cfg.data_dir.join("cuda"))
}

fn complete(dir: &Path) -> bool {
    REQUIRED.iter().all(|f| dir.join(f).is_file())
}

pub fn status(cfg: &Config) -> CudaStatus {
    let dir = dir(cfg);
    let system32 = std::env::var_os("SystemRoot").map(|r| PathBuf::from(r).join("System32")).unwrap_or_default();
    CudaStatus {
        supported: cfg!(feature = "cuda"),
        nvidia_gpu: system32.join("nvcuda.dll").is_file(),
        installed: complete(&dir),
        dir: dir.display().to_string(),
        download_bytes: WHEELS.iter().map(|w| w.bytes).sum(),
    }
}

/// Put the CUDA directory on this process's DLL search path. Must run before the first
/// ONNX Runtime session is created.
pub fn add_to_path(cfg: &Config) {
    let dir = dir(cfg);
    if complete(&dir) {
        let old = std::env::var_os("PATH").unwrap_or_default();
        let mut paths = vec![dir];
        paths.extend(std::env::split_paths(&old));
        if let Ok(joined) = std::env::join_paths(paths) {
            // SAFETY-equivalent note: called once at start-up, before any other thread exists.
            std::env::set_var("PATH", joined);
        }
    }
}

/// Download, verify and unpack the runtime. `progress` receives 0..=1.
pub async fn install(cfg: &Config, mut progress: impl FnMut(f64)) -> Result<()> {
    let target = dir(cfg);
    tokio::fs::create_dir_all(&target).await?;
    let tmp = cfg.tmp_dir();
    let total: u64 = WHEELS.iter().map(|w| w.bytes).sum();
    let mut done = 0u64;
    let client = reqwest::Client::builder().user_agent(concat!("ChordKu/", env!("CARGO_PKG_VERSION"))).build()?;

    for wheel in WHEELS {
        let path = tmp.join(format!("{}.whl.part", wheel.name));
        let mut out = tokio::fs::File::create(&path).await?;
        let mut hasher = Sha256::new();
        let mut got = 0u64;
        let mut stream = client.get(wheel.url).send().await?.error_for_status()?.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.context("unduhan terputus")?;
            got += chunk.len() as u64;
            if got > wheel.bytes {
                bail!("{} lebih besar dari yang diharapkan", wheel.name);
            }
            hasher.update(&chunk);
            out.write_all(&chunk).await?;
            progress((done + got) as f64 / total as f64 * 0.97);
        }
        out.flush().await?;
        drop(out);
        if got != wheel.bytes || format!("{:x}", hasher.finalize()) != wheel.sha256 {
            let _ = tokio::fs::remove_file(&path).await;
            bail!("{}: checksum tidak cocok", wheel.name);
        }
        let (archive, dest) = (path.clone(), target.clone());
        tokio::task::spawn_blocking(move || extract_dlls(&archive, &dest)).await??;
        let _ = tokio::fs::remove_file(&path).await;
        done += wheel.bytes;
    }
    if !complete(&target) {
        bail!("pustaka CUDA tidak lengkap setelah diekstrak");
    }
    progress(1.0);
    Ok(())
}

/// Keep only the DLLs of a wheel, flattened into `dest`. Entry names never become paths.
fn extract_dlls(wheel: &Path, dest: &Path) -> Result<()> {
    let mut zip = zip::ZipArchive::new(std::fs::File::open(wheel)?).context("wheel bukan arsip zip")?;
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i)?;
        let Some(name) = Path::new(entry.name()).file_name().and_then(|n| n.to_str()).map(str::to_owned) else { continue };
        let plain = name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'));
        if !plain || !name.to_ascii_lowercase().ends_with(".dll") || SKIP.contains(&name.as_str()) {
            continue;
        }
        let part = dest.join(format!("{name}.part"));
        let mut out = std::fs::File::create(&part)?;
        let mut limited = (&mut entry).take(2 * 1024 * 1024 * 1024);
        std::io::copy(&mut limited, &mut out)?;
        drop(out);
        std::fs::rename(&part, dest.join(&name))?;
    }
    Ok(())
}
