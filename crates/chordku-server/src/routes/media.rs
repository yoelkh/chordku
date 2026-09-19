//! Audio files with HTTP range support. Only names we generated ourselves are served,
//! from a directory derived from a validated id: no client-supplied path ever reaches
//! the file system.

use axum::extract::{Path, Request, State};
use axum::http::header::{self, HeaderValue};
use axum::response::{IntoResponse, Response};
use tower::ServiceExt;
use tower_http::services::ServeFile;

use crate::app::AppState;
use crate::error::ApiError;
use crate::routes::songs::valid_id;

const SERVABLE: &[&str] =
    &["mix.opus", "drums.opus", "bass.opus", "other.opus", "vocals.opus", "vocals_mono.opus", "backing_mono.opus"];

pub async fn serve(State(st): State<AppState>, Path((song_id, file)): Path<(String, String)>, req: Request) -> Response {
    if !valid_id(&song_id) || !SERVABLE.contains(&file.as_str()) {
        return ApiError::not_found().into_response();
    }
    let path = st.cfg.song_dir(&song_id).join(&file);
    if !path.is_file() {
        return ApiError::not_found().into_response();
    }
    match ServeFile::new(path).oneshot(req).await {
        Ok(mut res) => {
            res.headers_mut().insert(header::CONTENT_TYPE, HeaderValue::from_static("audio/ogg; codecs=opus"));
            // Files are replaced only by re-analysis; let the browser revalidate cheaply.
            res.headers_mut().insert(header::CACHE_CONTROL, HeaderValue::from_static("private, no-cache"));
            res.into_response()
        }
        Err(never) => match never {},
    }
}
