//! The web UI, embedded in the binary at build time (read from disk in debug builds so the
//! front end can be rebuilt without recompiling).

use axum::http::header::{self, HeaderValue};
use axum::http::{StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use rust_embed::Embed;

#[derive(Embed)]
#[folder = "$CARGO_MANIFEST_DIR/../../web/dist"]
struct Assets;

pub async fn serve(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    let (file, name) = match Assets::get(path) {
        Some(f) if !path.is_empty() => (f, path),
        // Single-page app: unknown paths are client-side routes.
        _ => match Assets::get("index.html") {
            Some(f) => (f, "index.html"),
            None => {
                return (StatusCode::NOT_FOUND, "UI belum dibangun: jalankan `npm run build` di folder web").into_response()
            }
        },
    };
    let mime = mime_guess::from_path(name).first_or_octet_stream();
    // Vite fingerprints everything under assets/, so those never change under a given name.
    let cache = if name.starts_with("assets/") { "public, max-age=31536000, immutable" } else { "no-cache" };
    let mut res = Response::new(axum::body::Body::from(file.data.into_owned()));
    let content_type =
        HeaderValue::from_str(mime.as_ref()).unwrap_or_else(|_| HeaderValue::from_static("application/octet-stream"));
    res.headers_mut().insert(header::CONTENT_TYPE, content_type);
    res.headers_mut().insert(header::CACHE_CONTROL, HeaderValue::from_static(cache));
    res
}
