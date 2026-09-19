//! Health, models and tools.

use axum::extract::{Path, State};
use axum::Json;
use serde_json::{json, Value};

use crate::app::AppState;
use crate::error::{ApiError, ApiResult};
use crate::jobs::Event;
use crate::ytdlp::YtDlp;
use crate::{models, platform};

fn dir_size(path: &std::path::Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(path) else { return 0 };
    entries
        .flatten()
        .map(|e| match e.metadata() {
            Ok(m) if m.is_dir() => dir_size(&e.path()),
            Ok(m) => m.len(),
            Err(_) => 0,
        })
        .sum()
}

pub async fn info(State(st): State<AppState>) -> ApiResult<Json<Value>> {
    let cfg = st.cfg.clone();
    let media_bytes = tokio::task::spawn_blocking(move || dir_size(&cfg.media_dir())).await.unwrap_or(0);
    Ok(Json(json!({
        "version": env!("CARGO_PKG_VERSION"),
        "onnxruntime": chordku_core::ort_version(),
        "data_dir": st.cfg.data_dir.display().to_string(),
        "media_bytes": media_bytes,
        "free_memory": platform::free_memory(),
        "max_duration_secs": st.cfg.max_duration_secs,
        "max_upload_bytes": st.cfg.max_upload_bytes,
        "models": models::status(&st.cfg),
        "accel": st.jobs.accel_state(),
        "cuda": crate::cuda::status(&st.cfg),
        "ytdlp": YtDlp::status(&st.cfg).await,
    })))
}

pub async fn download_model(State(st): State<AppState>, Path(id): Path<String>) -> ApiResult<Json<Value>> {
    let group = models::group(&id).ok_or_else(ApiError::not_found)?;
    if group.files.iter().any(|f| f.url.is_none()) {
        return Err(ApiError::conflict("Model ini dibuat lokal: jalankan tools\\setup.ps1."));
    }
    let state = st.clone();
    tokio::spawn(async move {
        let events = state.events.clone();
        let mut last = -1.0f64;
        let result = models::download(&state.cfg, &id, |p| {
            if p - last >= 0.01 || p >= 1.0 {
                last = p;
                let _ = events.send(Event::Model { id: id.clone(), progress: p, error: None, done: false });
            }
        })
        .await;
        let error = result.err().map(|e| format!("{e:#}"));
        if error.is_none() {
            // A new model changes what can be accelerated: measure it right away.
            state.jobs.request_calibration();
        }
        let _ = state.events.send(Event::Model { id: id.clone(), progress: 1.0, error, done: true });
    });
    Ok(Json(json!({ "started": true })))
}

/// Download NVIDIA's CUDA runtime (about 1.1 GB), then measure the devices again.
pub async fn install_cuda(State(st): State<AppState>) -> ApiResult<Json<Value>> {
    let status = crate::cuda::status(&st.cfg);
    if !status.supported || !status.nvidia_gpu {
        return Err(ApiError::conflict("Build ini atau laptop ini tidak mendukung CUDA (butuh GPU NVIDIA)."));
    }
    let state = st.clone();
    tokio::spawn(async move {
        let events = state.events.clone();
        let mut last = -1.0f64;
        let result = crate::cuda::install(&state.cfg, |p| {
            if p - last >= 0.005 || p >= 1.0 {
                last = p;
                let _ = events.send(Event::Model { id: "cuda".into(), progress: p, error: None, done: false });
            }
        })
        .await;
        let error = result.as_ref().err().map(|e| format!("{e:#}"));
        let _ = state.events.send(Event::Model { id: "cuda".into(), progress: 1.0, error, done: true });
    });
    Ok(Json(json!({ "started": true })))
}

pub async fn calibrate(State(st): State<AppState>) -> ApiResult<Json<Value>> {
    st.jobs.request_calibration();
    Ok(Json(json!({ "started": true })))
}

pub async fn update_ytdlp(State(st): State<AppState>) -> ApiResult<Json<Value>> {
    let tag = YtDlp::install(&st.cfg).await.map_err(|e| ApiError::bad_request(format!("{e:#}")))?;
    let _ = st.events.send(Event::System);
    Ok(Json(json!({ "version": tag, "status": YtDlp::status(&st.cfg).await })))
}
