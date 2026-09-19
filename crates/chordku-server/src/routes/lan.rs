//! LAN mode: status, pairing and devices. Administration is reachable from the laptop only
//! (the guard refuses `/api/system/*` on the LAN listener).

use axum::extract::{Path, State};
use axum::http::header::{self, HeaderValue};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::app::{self, AppState};
use crate::error::{ApiError, ApiResult};
use crate::jobs::Event;
use crate::lan;

fn payload(st: &AppState) -> ApiResult<Value> {
    Ok(json!({ "lan": st.lan.info(&st.db), "devices": st.db.devices()? }))
}

pub async fn info(State(st): State<AppState>) -> ApiResult<Json<Value>> {
    Ok(Json(payload(&st)?))
}

#[derive(Deserialize)]
pub struct EnableBody {
    enabled: bool,
}

pub async fn set_enabled(State(st): State<AppState>, Json(body): Json<EnableBody>) -> ApiResult<Json<Value>> {
    st.lan.set_enabled(&st.db, body.enabled)?;
    if body.enabled {
        st.lan.start(app::router(st.for_lan())).await.map_err(|e| ApiError::conflict(format!("{e:#}")))?;
        st.lan.new_pair_code();
    } else {
        st.lan.stop();
    }
    let _ = st.events.send(Event::System);
    Ok(Json(payload(&st)?))
}

pub async fn new_pair_code(State(st): State<AppState>) -> ApiResult<Json<Value>> {
    if !st.lan.info(&st.db).running {
        return Err(ApiError::conflict("Mode LAN belum aktif."));
    }
    st.lan.new_pair_code();
    Ok(Json(payload(&st)?))
}

pub async fn delete_device(State(st): State<AppState>, Path(id): Path<String>) -> ApiResult<Json<Value>> {
    if !st.db.delete_device(&id)? {
        return Err(ApiError::not_found());
    }
    Ok(Json(payload(&st)?))
}

/// What the front end needs to know before anything else: is this the laptop or a paired phone?
pub async fn session(State(st): State<AppState>, headers: axum::http::HeaderMap) -> Json<Value> {
    let paired = !st.via_lan
        || headers
            .get(header::COOKIE)
            .and_then(|c| c.to_str().ok())
            .and_then(|c| app::cookie_value(c, lan::COOKIE))
            .is_some_and(|t| st.db.touch_device(&lan::hash_token(t)).unwrap_or(false));
    Json(json!({ "lan": st.via_lan, "paired": paired }))
}

#[derive(Deserialize)]
pub struct PairBody {
    code: String,
    #[serde(default)]
    name: String,
}

pub async fn pair(State(st): State<AppState>, Json(body): Json<PairBody>) -> Response {
    if !st.via_lan {
        return ApiError::bad_request("Pemasangan hanya untuk perangkat di jaringan lokal.").into_response();
    }
    let Some(token) = st.lan.redeem(body.code.trim()) else {
        return ApiError::new(StatusCode::UNAUTHORIZED, "Kode pemasangan salah atau kedaluwarsa. Buat kode baru di laptop.").into_response();
    };
    let name: String = body.name.trim().chars().take(60).collect();
    let name = if name.is_empty() { "Perangkat".to_owned() } else { name };
    let id = ulid::Ulid::new().to_string();
    if let Err(e) = st.db.add_device(&id, &name, &lan::hash_token(&token)) {
        return ApiError::from(e).into_response();
    }
    let _ = st.events.send(Event::System);
    // HttpOnly: page scripts never see the token. Secure + SameSite=Strict: only sent over
    // TLS, and never on requests initiated by other sites.
    let cookie = format!("{}={token}; Path=/; Max-Age=31536000; HttpOnly; Secure; SameSite=Strict", lan::COOKIE);
    let mut res = Json(json!({ "ok": true, "name": name })).into_response();
    if let Ok(v) = HeaderValue::from_str(&cookie) {
        res.headers_mut().insert(header::SET_COOKIE, v);
    }
    res
}

/// The local CA certificate, for users who want to get rid of the browser warning.
pub async fn ca_cert(State(st): State<AppState>) -> Response {
    match tokio::fs::read(st.lan.ca_cert_path()).await {
        Ok(pem) => (
            [
                (header::CONTENT_TYPE, HeaderValue::from_static("application/x-x509-ca-cert")),
                (header::CONTENT_DISPOSITION, HeaderValue::from_static("attachment; filename=\"chordku-ca.crt\"")),
            ],
            pem,
        )
            .into_response(),
        Err(_) => ApiError::not_found().into_response(),
    }
}
