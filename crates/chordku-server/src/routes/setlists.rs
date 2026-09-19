//! Setlists: ordered groups of songs for a rehearsal or a gig.

use axum::extract::{Path, State};
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
