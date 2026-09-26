//! Library, ingest and chart endpoints.

use std::path::Path as FsPath;

use axum::extract::{Multipart, Path, Query, State};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};
use tokio::io::AsyncWriteExt;

use crate::app::AppState;
use crate::db::{Edit, NewSong};
use crate::error::{ApiError, ApiResult};
use crate::jobs::Event;
use crate::ytdlp::{self, YtDlp};

const EDIT_OPS: &[&str] = &[
    "set_chord",
    "delete",
    "split",
    "key_override",
    "meter_override",
    "downbeat_shift",
    "tempo_scale",
    "section_rename",
];
const AUDIO_EXTENSIONS: &[&str] =
    &["mp3", "wav", "flac", "m4a", "aac", "ogg", "opus", "oga", "webm", "mp4", "mkv", "wma", "aiff", "aif", "3gp", "amr", "mov"];

pub fn valid_id(id: &str) -> bool {
    id.len() == 26 && id.bytes().all(|b| b.is_ascii_alphanumeric())
}

fn check_id(id: &str) -> ApiResult<()> {
    if valid_id(id) {
        Ok(())
    } else {
        Err(ApiError::not_found())
    }
}

#[derive(Deserialize)]
pub struct ListQuery {
    q: Option<String>,
    tag: Option<String>,
    fav: Option<u8>,
}

pub async fn list(State(st): State<AppState>, Query(query): Query<ListQuery>) -> ApiResult<Json<Value>> {
    let songs = st.db.songs(query.q.as_deref(), query.tag.as_deref(), query.fav == Some(1))?;
    Ok(Json(json!({ "songs": songs, "tags": st.db.all_tags()? })))
}

fn song_payload(st: &AppState, id: &str) -> ApiResult<Value> {
    let song = st.db.song(id)?.ok_or_else(ApiError::not_found)?;
    let analysis: Option<Value> = st.db.active_analysis(id)?.and_then(|j| serde_json::from_str(&j).ok());
    let jobs: Vec<_> = st.db.recent_jobs()?.into_iter().filter(|j| j.song_id == id).collect();
    Ok(json!({
        "song": song,
        "analysis": analysis,
        "edits": st.db.edits(id)?,
        "prefs": st.db.prefs(id)?,
        "stems": st.db.stems(id)?,
        "drums": st.db.drums(id)?.and_then(|j| serde_json::from_str::<Value>(&j).ok()),
        "jobs": jobs,
    }))
}

pub async fn get(State(st): State<AppState>, Path(id): Path<String>) -> ApiResult<Json<Value>> {
    check_id(&id)?;
    Ok(Json(song_payload(&st, &id)?))
}

#[derive(Deserialize)]
pub struct UrlBody {
    url: String,
}

pub async fn create_from_url(State(st): State<AppState>, Json(body): Json<UrlBody>) -> ApiResult<Json<Value>> {
    let url = ytdlp::validate_url(&body.url).await.map_err(|e| ApiError::bad_request(e.to_string()))?;
    let yt = match YtDlp::locate(&st.cfg).await {
        Some(y) => y,
        None => {
            // First URL ever: fetch the official yt-dlp build (checksum verified).
            YtDlp::install(&st.cfg).await.map_err(|e| ApiError::bad_request(format!("Gagal memasang yt-dlp: {e:#}")))?;
            YtDlp::locate(&st.cfg).await.ok_or_else(|| ApiError::bad_request("yt-dlp tidak bisa dijalankan"))?
        }
    };
    let meta = yt.probe(&url).await.map_err(|e| ApiError::bad_request(e.to_string()))?;
    if meta.is_live {
        return Err(ApiError::bad_request("Siaran langsung tidak didukung."));
    }
    if let Some(d) = meta.duration_secs.filter(|d| *d > st.cfg.max_duration_secs as f64) {
        return Err(ApiError::bad_request(format!(
            "Durasi {:.0} menit melebihi batas {} menit.",
            d / 60.0,
            st.cfg.max_duration_secs / 60
        )));
    }
    if let Some(existing) = st.db.find_by_media(&meta.extractor, &meta.id)? {
        return Ok(Json(json!({ "existing": true, "song": st.db.song(&existing)? })));
    }

    let id = ulid::Ulid::new().to_string();
    st.db.insert_song(
        &id,
        &NewSong {
            title: meta.title.clone(),
            artist: meta.artist.clone(),
            src_kind: "url".into(),
            src_url: Some(if meta.webpage_url.is_empty() { url.to_string() } else { meta.webpage_url.clone() }),
            extractor: Some(meta.extractor.clone()),
            media_id: Some(meta.id.clone()),
            thumbnail: meta.thumbnail.clone(),
            duration_ms: meta.duration_secs.map(|d| (d * 1000.0) as u32).unwrap_or(0),
        },
    )?;
    let job = st.jobs.create(&id, "analyze")?;

    // Download on the async runtime, then hand over to the inference thread.
    let (state, song_id, job_clone) = (st.clone(), id.clone(), job.clone());
    tokio::spawn(async move {
        let jobs = state.jobs.clone();
        let result: anyhow::Result<()> = async {
            let dir = state.cfg.song_dir(&song_id);
            tokio::fs::create_dir_all(&dir).await?;
            jobs.report(&job_clone.id, "running", "download", 0.0, None);
            let mut last = -1.0f64;
            let file = yt
                .download(&url, &dir, "source", |p| {
                    if p - last >= 0.02 || p >= 1.0 {
                        last = p;
                        jobs.report(&job_clone.id, "running", "download", p, None);
                    }
                })
                .await?;
            if jobs.is_canceled(&job_clone.id) {
                jobs.report(&job_clone.id, "canceled", "", 0.0, None);
                return Ok(());
            }
            let name = file.file_name().and_then(|n| n.to_str()).unwrap_or("source").to_owned();
            let hash = tokio::task::spawn_blocking(move || crate::jobs::hash_file(&file)).await??;
            state.db.set_song_source(&song_id, &name, &hash, 0)?;
            jobs.report(&job_clone.id, "queued", "", 0.0, None);
            jobs.enqueue(&job_clone);
            Ok(())
        }
        .await;
        if let Err(e) = result {
            state.jobs.report(&job_clone.id, "failed", "", 0.0, Some(&format!("{e:#}")));
        }
    });

    Ok(Json(json!({ "existing": false, "song": st.db.song(&id)?, "job": job })))
}

/// Multipart upload: a `file` part plus optional `title`, `artist` and `kind` (`file` | `mic`).
pub async fn upload(State(st): State<AppState>, mut multipart: Multipart) -> ApiResult<Json<Value>> {
    let id = ulid::Ulid::new().to_string();
    let tmp = st.cfg.tmp_dir().join(format!("{id}.upload"));
    let mut fields = std::collections::HashMap::<String, String>::new();
    let mut original_name = String::new();
    let mut received = 0u64;
    let mut hasher = blake3::Hasher::new();

    let outcome: ApiResult<()> = async {
        while let Some(mut field) = multipart.next_field().await.map_err(|e| ApiError::bad_request(e.to_string()))? {
            let name = field.name().unwrap_or("").to_owned();
            if name != "file" {
                let text = field.text().await.map_err(|e| ApiError::bad_request(e.to_string()))?;
                fields.insert(name, text.chars().take(300).collect());
                continue;
            }
            original_name = field.file_name().unwrap_or("rekaman").to_owned();
            let mut out = tokio::fs::File::create(&tmp).await.map_err(anyhow::Error::from)?;
            while let Some(chunk) = field.chunk().await.map_err(|e| ApiError::bad_request(e.to_string()))? {
                received += chunk.len() as u64;
                if received > st.cfg.max_upload_bytes {
                    return Err(ApiError::new(axum::http::StatusCode::PAYLOAD_TOO_LARGE, "File terlalu besar."));
                }
                hasher.update(&chunk);
                out.write_all(&chunk).await.map_err(anyhow::Error::from)?;
            }
            out.flush().await.map_err(anyhow::Error::from)?;
        }
        if received == 0 {
            return Err(ApiError::bad_request("Tidak ada file yang diunggah."));
        }
        Ok(())
    }
    .await;
    if let Err(e) = outcome {
        let _ = tokio::fs::remove_file(&tmp).await;
        return Err(e);
    }

    let hash = hasher.finalize().to_hex().to_string();
    if let Some(existing) = st.db.find_by_hash(&hash)? {
        let _ = tokio::fs::remove_file(&tmp).await;
        return Ok(Json(json!({ "existing": true, "song": st.db.song(&existing)? })));
    }

    // Never trust the client's file name for a path: keep only a known-safe extension.
    let ext = FsPath::new(&original_name)
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .filter(|e| AUDIO_EXTENSIONS.contains(&e.as_str()))
        .unwrap_or_else(|| "bin".into());
    let source = format!("source.{ext}");
    let dir = st.cfg.song_dir(&id);
    tokio::fs::create_dir_all(&dir).await.map_err(anyhow::Error::from)?;
    tokio::fs::rename(&tmp, dir.join(&source)).await.map_err(anyhow::Error::from)?;

    let kind = if fields.get("kind").map(String::as_str) == Some("mic") { "mic" } else { "file" };
    let fallback_title = FsPath::new(&original_name).file_stem().and_then(|s| s.to_str()).unwrap_or("Tanpa judul").to_owned();
    let title = fields.get("title").filter(|t| !t.trim().is_empty()).cloned().unwrap_or(fallback_title);
    st.db.insert_song(
        &id,
        &NewSong {
            title,
            artist: fields.get("artist").cloned().unwrap_or_default(),
            src_kind: kind.into(),
            ..Default::default()
        },
    )?;
    st.db.set_song_source(&id, &source, &hash, 0)?;
    let job = st.jobs.create(&id, "analyze")?;
    st.jobs.enqueue(&job);
    Ok(Json(json!({ "existing": false, "song": st.db.song(&id)?, "job": job })))
}

#[derive(Deserialize)]
pub struct PatchBody {
    title: Option<String>,
    artist: Option<String>,
    fav: Option<bool>,
    tags: Option<Vec<String>>,
}

pub async fn patch(State(st): State<AppState>, Path(id): Path<String>, Json(body): Json<PatchBody>) -> ApiResult<Json<Value>> {
    check_id(&id)?;
    st.db.song(&id)?.ok_or_else(ApiError::not_found)?;
    let clean = |s: &String| s.trim().chars().take(300).collect::<String>();
    let title = body.title.as_ref().map(clean).filter(|t| !t.is_empty());
    st.db.update_song_meta(&id, title.as_deref(), body.artist.as_ref().map(clean).as_deref(), body.fav)?;
    if let Some(tags) = &body.tags {
        let tags: Vec<String> = tags.iter().map(|t| t.trim().chars().take(40).collect()).take(20).collect();
        st.db.set_tags(&id, &tags)?;
    }
    let _ = st.events.send(Event::Song { id: id.clone() });
    Ok(Json(json!({ "song": st.db.song(&id)? })))
}

pub async fn remove(State(st): State<AppState>, Path(id): Path<String>) -> ApiResult<Json<Value>> {
    check_id(&id)?;
    for job in st.db.recent_jobs()?.into_iter().filter(|j| j.song_id == id) {
        st.jobs.cancel(&job.id)?;
    }
    if !st.db.delete_song(&id)? {
        return Err(ApiError::not_found());
    }
    let dir = st.cfg.song_dir(&id);
    if dir.starts_with(st.cfg.media_dir()) {
        let _ = tokio::fs::remove_dir_all(&dir).await;
    }
    let _ = st.events.send(Event::SongDeleted { id });
    Ok(Json(json!({ "ok": true })))
}

pub async fn add_edit(State(st): State<AppState>, Path(id): Path<String>, Json(edit): Json<Edit>) -> ApiResult<Json<Value>> {
    check_id(&id)?;
    st.db.song(&id)?.ok_or_else(ApiError::not_found)?;
    if !EDIT_OPS.contains(&edit.op.as_str()) {
        return Err(ApiError::bad_request(format!("operasi edit tidak dikenal: {}", edit.op)));
    }
    if edit.t1_ms < edit.t0_ms || edit.payload.to_string().len() > 2000 {
        return Err(ApiError::bad_request("edit tidak valid"));
    }
    let edit_id = st.db.add_edit(&id, &edit)?;
    let _ = st.events.send(Event::Song { id });
    Ok(Json(json!({ "id": edit_id })))
}

pub async fn delete_edit(State(st): State<AppState>, Path((id, edit_id)): Path<(String, i64)>) -> ApiResult<Json<Value>> {
    check_id(&id)?;
    if !st.db.delete_edit(&id, edit_id)? {
        return Err(ApiError::not_found());
    }
    let _ = st.events.send(Event::Song { id });
    Ok(Json(json!({ "ok": true })))
}

pub async fn clear_edits(State(st): State<AppState>, Path(id): Path<String>) -> ApiResult<Json<Value>> {
    check_id(&id)?;
    st.db.clear_edits(&id)?;
    let _ = st.events.send(Event::Song { id });
    Ok(Json(json!({ "ok": true })))
}

pub async fn set_prefs(State(st): State<AppState>, Path(id): Path<String>, Json(prefs): Json<Value>) -> ApiResult<Json<Value>> {
    check_id(&id)?;
    st.db.song(&id)?.ok_or_else(ApiError::not_found)?;
    if !prefs.is_object() || prefs.to_string().len() > 4000 {
        return Err(ApiError::bad_request("prefs tidak valid"));
    }
    st.db.set_prefs(&id, &prefs)?;
    Ok(Json(json!({ "ok": true })))
}

fn start_job(st: &AppState, id: &str, kind: &str) -> ApiResult<Json<Value>> {
    check_id(id)?;
    let song = st.db.song(id)?.ok_or_else(ApiError::not_found)?;
    if song.source_file.is_none() {
        return Err(ApiError::conflict("Audio lagu ini belum selesai diunduh."));
    }
    if let Some(existing) = st.db.active_job_for(id, kind)? {
        return Ok(Json(json!({ "job": st.db.job(&existing)? })));
    }
    let job = st.jobs.create(id, kind)?;
    st.jobs.enqueue(&job);
    Ok(Json(json!({ "job": job })))
}

pub async fn reanalyze(State(st): State<AppState>, Path(id): Path<String>) -> ApiResult<Json<Value>> {
    start_job(&st, &id, "analyze")
}

pub async fn separate(State(st): State<AppState>, Path(id): Path<String>) -> ApiResult<Json<Value>> {
    if crate::models::group_dir(&st.cfg, "stems").is_none() {
        return Err(ApiError::conflict("Model pemisah stem belum diunduh. Buka Pengaturan lalu Model."));
    }
    start_job(&st, &id, "separate")
}

pub async fn delete_stems(State(st): State<AppState>, Path(id): Path<String>) -> ApiResult<Json<Value>> {
    check_id(&id)?;
    let (keep, drop): (Vec<_>, Vec<_>) = st.db.stems(&id)?.into_iter().partition(|s| s.name == "mix");
    let dir = st.cfg.song_dir(&id);
    for stem in drop {
        let _ = tokio::fs::remove_file(dir.join(&stem.file)).await;
    }
    st.db.replace_stems(&id, &keep)?;
    let _ = st.events.send(Event::Song { id });
    Ok(Json(json!({ "ok": true })))
}

pub async fn cancel_job(State(st): State<AppState>, Path(id): Path<String>) -> ApiResult<Json<Value>> {
    check_id(&id)?;
    Ok(Json(json!({ "canceled": st.jobs.cancel(&id)? })))
}
