//! Setlists: ordered groups of songs for a rehearsal or a gig.

use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::app::AppState;
use crate::error::{ApiError, ApiResult};
use crate::routes::songs::valid_id;

#[derive(Deserialize)]
pub struct NameBody {
    name: String,
}

fn clean_name(raw: &str) -> ApiResult<String> {
    let name: String = raw.trim().chars().take(80).collect();
    if name.is_empty() {
        Err(ApiError::bad_request("Nama setlist kosong."))
    } else {
        Ok(name)
    }
}

pub async fn list(State(st): State<AppState>) -> ApiResult<Json<Value>> {
    Ok(Json(json!({ "setlists": st.db.setlists()? })))
}

pub async fn create(State(st): State<AppState>, Json(body): Json<NameBody>) -> ApiResult<Json<Value>> {
    let id = st.db.create_setlist(&clean_name(&body.name)?)?;
    Ok(Json(json!({ "id": id, "setlists": st.db.setlists()? })))
}

pub async fn get(State(st): State<AppState>, Path(id): Path<i64>) -> ApiResult<Json<Value>> {
    let setlist = st.db.setlists()?.into_iter().find(|l| l.id == id).ok_or_else(ApiError::not_found)?;
    Ok(Json(json!({ "setlist": setlist, "songs": st.db.setlist_songs(id)? })))
}

pub async fn rename(State(st): State<AppState>, Path(id): Path<i64>, Json(body): Json<NameBody>) -> ApiResult<Json<Value>> {
    if !st.db.rename_setlist(id, &clean_name(&body.name)?)? {
        return Err(ApiError::not_found());
    }
    Ok(Json(json!({ "setlists": st.db.setlists()? })))
}

pub async fn remove(State(st): State<AppState>, Path(id): Path<i64>) -> ApiResult<Json<Value>> {
    if !st.db.delete_setlist(id)? {
        return Err(ApiError::not_found());
    }
    Ok(Json(json!({ "setlists": st.db.setlists()? })))
}

#[derive(Deserialize)]
pub struct OrderBody {
    song_ids: Vec<String>,
}

pub async fn set_songs(State(st): State<AppState>, Path(id): Path<i64>, Json(body): Json<OrderBody>) -> ApiResult<Json<Value>> {
    if body.song_ids.len() > 500 || !body.song_ids.iter().all(|s| valid_id(s)) {
        return Err(ApiError::bad_request("daftar lagu tidak valid"));
    }
    st.db.set_setlist_songs(id, &body.song_ids)?;
    Ok(Json(json!({ "songs": st.db.setlist_songs(id)? })))
}

#[derive(Deserialize)]
pub struct AddBody {
    song_id: String,
}

pub async fn add_song(State(st): State<AppState>, Path(id): Path<i64>, Json(body): Json<AddBody>) -> ApiResult<Json<Value>> {
    if !valid_id(&body.song_id) {
        return Err(ApiError::bad_request("lagu tidak valid"));
    }
    st.db.append_to_setlist(id, &body.song_id)?;
    Ok(Json(json!({ "setlists": st.db.setlists()? })))
}

// ---- sharing: the setlist as one self-contained web page ----------------------------------

/// Everything the standalone viewer needs: songs in set order with their analysis, edits and
/// the owner's view prefs. Songs without a chart are left out.
#[derive(Deserialize, Default)]
pub struct ShareQuery {
    /// Embed each song's playback file (`mix.opus`) as a data URI: a much bigger file, but the
    /// page then plays the song like the app does, from any folder or chat attachment.
    #[serde(default)]
    audio: Option<String>,
}

impl ShareQuery {
    fn audio(&self) -> bool {
        matches!(self.audio.as_deref(), Some("1" | "true" | "yes"))
    }
}

fn share_data(st: &AppState, id: i64, with_audio: bool) -> ApiResult<Value> {
    let setlist = st.db.setlists()?.into_iter().find(|l| l.id == id).ok_or_else(ApiError::not_found)?;
    let mut songs = Vec::new();
    for song in st.db.setlist_songs(id)? {
        let Some(analysis) = st.db.active_analysis(&song.id)? else { continue };
        let analysis: Value = serde_json::from_str(&analysis).map_err(anyhow::Error::from)?;
        let audio = if with_audio {
            let mix = st.cfg.song_dir(&song.id).join(crate::jobs::MIX_FILE);
            match std::fs::read(&mix) {
                Ok(bytes) => {
                    use base64::Engine as _;
                    json!({ "mix": format!("data:audio/ogg;base64,{}", base64::engine::general_purpose::STANDARD.encode(bytes)) })
                }
                Err(_) => Value::Null,
            }
        } else {
            Value::Null
        };
        songs.push(json!({
            "audio": audio,
            "id": song.id,
            "title": song.title,
            "artist": song.artist,
            "duration_ms": song.duration_ms,
            "src_url": song.src_url,
            "thumbnail": song.thumbnail,
            "analysis": analysis,
            "edits": st.db.edits(&song.id)?,
            "drums": st.db.drums(&song.id)?.and_then(|j| serde_json::from_str::<Value>(&j).ok()),
            "prefs": st.db.prefs(&song.id)?,
        }));
    }
    Ok(json!({
        "app": "ChordKu",
        "version": env!("CARGO_PKG_VERSION"),
        "setlist": { "id": setlist.id, "name": setlist.name },
        "exported_at": std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0),
        "songs": songs,
    }))
}

pub async fn share_json(State(st): State<AppState>, Path(id): Path<i64>, Query(q): Query<ShareQuery>) -> ApiResult<Json<Value>> {
    Ok(Json(share_data(&st, id, q.audio())?))
}

/// `viewer.html` (built with the UI) with the setlist embedded, served as a download. The
/// file needs no server afterwards: open it from disk, send it over chat, or put it on any
/// static host.
pub async fn share_html(State(st): State<AppState>, Path(id): Path<i64>, Query(q): Query<ShareQuery>) -> ApiResult<Response> {
    let data = share_data(&st, id, q.audio())?;
    let name = data["setlist"]["name"].as_str().unwrap_or("setlist").to_owned();
    let template = crate::routes::ui::Assets::get("viewer.html")
        .ok_or_else(|| ApiError::bad_request("viewer.html belum dibangun: jalankan `npm run build` di folder web"))?;
    let template = String::from_utf8_lossy(&template.data);
    // "</" must not end the script element early; "<\/" is the same string to JSON.
    let json = serde_json::to_string(&data).map_err(anyhow::Error::from)?.replace("</", "<\\/");
    let page = template.replace(
        "<!--CHORDKU_SETLIST-->",
        &format!("<script type=\"application/json\" id=\"chordku-setlist\">{json}</script>"),
    );
    let slug = slugify(&name) + if q.audio() { "-audio" } else { "" };
    let disposition = format!("attachment; filename=\"{slug}.html\"; filename*=UTF-8''{}", percent_encode(&format!("{name}{}.html", if q.audio() { " (audio)" } else { "" })));
    Ok((
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, HeaderValue::from_static("text/html; charset=utf-8")),
            (header::CONTENT_DISPOSITION, HeaderValue::from_str(&disposition).unwrap_or_else(|_| HeaderValue::from_static("attachment"))),
            (header::CACHE_CONTROL, HeaderValue::from_static("no-store")),
        ],
        page,
    )
        .into_response())
}

/// ASCII file-name stem: letters, digits and dashes.
pub fn slugify(name: &str) -> String {
    let mut out = String::new();
    let mut dash = false;
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
            dash = false;
        } else if !dash && !out.is_empty() {
            out.push('-');
            dash = true;
        }
    }
    let out = out.trim_end_matches('-').to_owned();
    if out.is_empty() { "setlist".into() } else { out }
}

fn percent_encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slug_is_ascii_and_dashed() {
        assert_eq!(slugify("Ibadah Minggu 28/09/2026"), "ibadah-minggu-28-09-2026");
        assert_eq!(slugify("  Sesi 1: Pujian & Penyembahan "), "sesi-1-pujian-penyembahan");
        assert_eq!(slugify("???"), "setlist");
    }
}
