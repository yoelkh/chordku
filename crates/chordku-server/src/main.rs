//! ChordKu server: `chordku [--open]`, then browse to http://127.0.0.1:7464

mod app;
mod config;
mod cuda;
mod db;
mod error;
mod jobs;
mod lan;
mod models;
mod platform;
mod routes;
mod ytdlp;

use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::{Context, Result};
use tokio::sync::broadcast;

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "chordku=info,chordku_core=info,ort=error".into()),
        )
        .with_target(false)
        .init();

    // Inference has its own thread and ONNX Runtime its own pool; the async side only moves
    // bytes, so two workers are plenty and leave the cores to the models.
    tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build()?.block_on(run())
}

async fn run() -> Result<()> {
    let cfg = config::Config::from_env()?;
    // Before any ONNX Runtime session exists: make NVIDIA's runtime DLLs findable.
    cuda::add_to_path(&cfg);
    let db = db::Db::open(&cfg.db_path())?;
    let (events, _) = broadcast::channel(256);
    let jobs = jobs::Jobs::start(cfg.clone(), db.clone(), events.clone())?;
    let cfg_arc = Arc::new(cfg.clone());
    let state = app::AppState {
        cfg: cfg_arc.clone(),
        db: db.clone(),
        jobs: jobs.clone(),
        events,
        lan: lan::Lan::new(cfg_arc),
        via_lan: false,
    };

    // ring instead of aws-lc: no NASM / CMake needed to build on Windows.
    let _ = rustls::crypto::ring::default_provider().install_default();
    if state.lan.is_enabled(&db) {
        if let Err(e) = state.lan.start(app::router(state.for_lan())).await {
            tracing::warn!("mode LAN tidak bisa dimulai: {e:#}");
        }
    }

    let addr = SocketAddr::new(cfg.bind, cfg.port);
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("port {} sedang dipakai (ChordKu sudah berjalan?)", cfg.port))?;
    let url = format!("http://127.0.0.1:{}", cfg.port);
    tracing::info!("ChordKu {} siap di {url}", env!("CARGO_PKG_VERSION"));
    tracing::info!("data: {}", cfg.data_dir.display());
    tracing::info!("{}", chordku_core::ort_version());
    if std::env::args().any(|a| a == "--open") {
        open_browser(&url);
    }

    axum::serve(listener, app::router(state))
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    jobs.shutdown();
    Ok(())
}

fn open_browser(url: &str) {
    #[cfg(windows)]
    let _ = std::process::Command::new("cmd").args(["/C", "start", "", url]).spawn();
    #[cfg(not(windows))]
    let _ = std::process::Command::new("xdg-open").arg(url).spawn();
}
