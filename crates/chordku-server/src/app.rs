//! Shared state, router and the request guards.

use std::net::SocketAddr;
use std::sync::Arc;

use axum::extract::{ConnectInfo, DefaultBodyLimit, Request, State};
use axum::http::header::{self, HeaderValue};
use axum::http::{Method, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post, put};
use axum::{Json, Router};
use tokio::sync::broadcast;
use tower_http::set_header::SetResponseHeaderLayer;

use crate::config::Config;
use crate::db::Db;
use crate::jobs::{Event, Jobs};
use crate::lan::{self, Lan};
use crate::routes;

#[derive(Clone)]
pub struct AppState {
    pub cfg: Arc<Config>,
    pub db: Db,
    pub jobs: Jobs,
    pub events: broadcast::Sender<Event>,
    pub lan: Lan,
    /// True for the router mounted on the HTTPS LAN listener.
    pub via_lan: bool,
}

impl AppState {
    /// The same application as seen from the LAN listener.
    pub fn for_lan(&self) -> Self {
        Self { via_lan: true, ..self.clone() }
    }
}

pub fn router(state: AppState) -> Router {
    let upload_limit = state.cfg.max_upload_bytes as usize + 1024 * 1024;
    let api = Router::new()
        .route("/songs", get(routes::songs::list))
        .route("/songs/url", post(routes::songs::create_from_url))
        .route("/songs/upload", post(routes::songs::upload).layer(DefaultBodyLimit::max(upload_limit)))
        .route("/songs/{id}", get(routes::songs::get).patch(routes::songs::patch).delete(routes::songs::remove))
        .route("/songs/{id}/edits", post(routes::songs::add_edit).delete(routes::songs::clear_edits))
        .route("/songs/{id}/edits/{edit_id}", delete(routes::songs::delete_edit))
        .route("/songs/{id}/prefs", put(routes::songs::set_prefs))
        .route("/songs/{id}/analyze", post(routes::songs::reanalyze))
        .route("/songs/{id}/separate", post(routes::songs::separate))
        .route("/songs/{id}/stems", delete(routes::songs::delete_stems))
        .route("/jobs/{id}/cancel", post(routes::songs::cancel_job))
        .route("/setlists", get(routes::setlists::list).post(routes::setlists::create))
        .route(
            "/setlists/{id}",
            get(routes::setlists::get).patch(routes::setlists::rename).delete(routes::setlists::remove),
        )
        .route("/setlists/{id}/songs", put(routes::setlists::set_songs).post(routes::setlists::add_song))
        .route("/events", get(routes::events::stream))
        .route("/media/{song_id}/{file}", get(routes::media::serve))
        .route("/session", get(routes::lan::session))
        .route("/pair", post(routes::lan::pair))
        .route("/system", get(routes::system::info))
        .route("/system/models/{id}/download", post(routes::system::download_model))
        .route("/system/ytdlp/update", post(routes::system::update_ytdlp))
        .route("/system/cuda/install", post(routes::system::install_cuda))
        .route("/system/accel/calibrate", post(routes::system::calibrate))
        .route("/system/lan", get(routes::lan::info).post(routes::lan::set_enabled))
        .route("/system/lan/pair-code", post(routes::lan::new_pair_code))
        .route("/system/lan/devices/{id}", delete(routes::lan::delete_device))
        .fallback(|| async { (StatusCode::NOT_FOUND, "unknown API route") });

    Router::new()
        .nest("/api", api)
        .route("/ca.crt", get(routes::lan::ca_cert))
        .fallback(routes::ui::serve)
        .layer(middleware::from_fn_with_state(state.clone(), guard))
        .layer(SetResponseHeaderLayer::if_not_present(header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff")))
        .layer(SetResponseHeaderLayer::if_not_present(header::REFERRER_POLICY, HeaderValue::from_static("no-referrer")))
        .layer(SetResponseHeaderLayer::if_not_present(
            header::CONTENT_SECURITY_POLICY,
            // Own scripts only, plus WASM and the blob: AudioWorklet module the time-stretcher
            // builds for itself (blob URLs can only be minted by our own scripts; inline and
            // eval stay forbidden). Thumbnails from https, no framing.
            HeaderValue::from_static(
                "default-src 'self'; script-src 'self' blob: 'wasm-unsafe-eval'; style-src 'self' 'unsafe-inline'; \
                 img-src 'self' data: https:; media-src 'self' blob:; connect-src 'self'; worker-src 'self' blob:; \
                 frame-ancestors 'none'; base-uri 'none'; form-action 'self'",
            ),
        ))
        .with_state(state)
}

/// Split `name[:port]`, where name may be a bracketed IPv6 literal.
fn split_host(host: &str) -> (&str, Option<&str>) {
    match host.strip_prefix('[').and_then(|rest| rest.find(']')) {
        Some(end) => (&host[..end + 2], host[end + 2..].strip_prefix(':')),
        None => match host.rsplit_once(':') {
            Some((n, p)) => (n, Some(p)),
            None => (host, None),
        },
    }
}

/// Hosts the loopback listener answers to. A page on the internet can make a browser
/// resolve its own domain to 127.0.0.1 (DNS rebinding); refusing unknown `Host` values
/// shuts that door.
fn host_allowed(host: &str, port: u16) -> bool {
    let (name, port_text) = split_host(host);
    if let Some(p) = port_text {
        if p.parse::<u16>().ok() != Some(port) {
            return false;
        }
    }
    matches!(name, "localhost" | "127.0.0.1" | "[::1]")
}

fn lan_host_allowed(lan: &Lan, host: &str) -> bool {
    let (name, port_text) = split_host(host);
    // 443 is never used, so the port is always explicit in a genuine request.
    if port_text.and_then(|p| p.parse::<u16>().ok()) != Some(lan.port()) {
        return false;
    }
    // DNS rebinding needs a domain name that the attacker controls. A literal private IPv4
    // address cannot be one, so any of them is fine: laptops with several interfaces (Wi-Fi,
    // hotspot, VPN) are reachable under addresses the start-up probe did not list.
    name.parse::<std::net::Ipv4Addr>().is_ok_and(|ip| ip.is_private()) || lan.allowed_host(name)
}

pub fn cookie_value<'a>(req_cookies: &'a str, name: &str) -> Option<&'a str> {
    req_cookies.split(';').filter_map(|c| c.trim().split_once('=')).find(|(k, _)| *k == name).map(|(_, v)| v)
}

fn deny(status: StatusCode, message: &str) -> Response {
    (status, Json(serde_json::json!({ "error": message }))).into_response()
}

/// The host the client addressed. HTTP/2 (what phone browsers speak over TLS) has no Host
/// header: the name travels in the `:authority` pseudo-header, which hyper exposes as the
/// URI authority. Reading only the header made every HTTP/2 request fail the allow-list.
fn request_host<B>(req: &axum::http::Request<B>) -> &str {
    req.headers()
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .or_else(|| req.uri().authority().map(|a| a.as_str()))
        .unwrap_or("")
}

async fn guard(State(state): State<AppState>, req: Request, next: Next) -> Response {
    let host = request_host(&req).to_owned();
    let host_ok = |h: &str| if state.via_lan { lan_host_allowed(&state.lan, h) } else { host_allowed(h, state.cfg.port) };
    if !host_ok(&host) {
        return deny(StatusCode::FORBIDDEN, "host not allowed");
    }
    // Cross-site requests: browsers attach Origin to every state-changing request.
    if !matches!(*req.method(), Method::GET | Method::HEAD | Method::OPTIONS) {
        if let Some(origin) = req.headers().get(header::ORIGIN).and_then(|o| o.to_str().ok()) {
            let origin_host = origin.split_once("://").map(|(_, h)| h).unwrap_or("");
            if !host_ok(origin_host) {
                return deny(StatusCode::FORBIDDEN, "cross-origin request refused");
            }
        }
    }

    if state.via_lan {
        let peer = req.extensions().get::<ConnectInfo<SocketAddr>>().map(|c| c.0.ip());
        if !peer.is_some_and(lan::is_private_peer) {
            return deny(StatusCode::FORBIDDEN, "hanya perangkat di jaringan lokal");
        }
        let path = req.uri().path();
        if path.starts_with("/api/") && path != "/api/pair" && path != "/api/session" {
            // Installing software, models and LAN administration stay on the laptop itself.
            if path.starts_with("/api/system") {
                return deny(StatusCode::FORBIDDEN, "Pengaturan ini hanya bisa diubah dari laptop.");
            }
            let paired = req
                .headers()
                .get(header::COOKIE)
                .and_then(|c| c.to_str().ok())
                .and_then(|c| cookie_value(c, lan::COOKIE))
                .is_some_and(|token| state.db.touch_device(&lan::hash_token(token)).unwrap_or(false));
            if !paired {
                return deny(StatusCode::UNAUTHORIZED, "Perangkat ini belum dipasangkan. Pindai kode QR dari laptop.");
            }
        }
    }
    next.run(req).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_header_allow_list() {
        assert!(host_allowed("localhost:7464", 7464));
        assert!(host_allowed("127.0.0.1:7464", 7464));
        assert!(host_allowed("[::1]:7464", 7464));
        assert!(host_allowed("localhost", 7464));
        assert!(!host_allowed("localhost:9999", 7464));
        assert!(!host_allowed("evil.example:7464", 7464));
        assert!(!host_allowed("127.0.0.1.evil.example:7464", 7464));
        assert!(!host_allowed("", 7464));
    }

    #[test]
    fn host_comes_from_the_header_or_the_http2_authority() {
        let h1 = axum::http::Request::builder().uri("/api/session").header("host", "192.168.1.9:7465").body(()).unwrap();
        assert_eq!(request_host(&h1), "192.168.1.9:7465");
        // HTTP/2: absolute-form URI, no Host header.
        let h2 = axum::http::Request::builder().uri("https://192.168.1.9:7465/api/session").body(()).unwrap();
        assert_eq!(request_host(&h2), "192.168.1.9:7465");
        let none = axum::http::Request::builder().uri("/x").body(()).unwrap();
        assert_eq!(request_host(&none), "");
    }

    #[test]
    fn cookies() {
        assert_eq!(cookie_value("a=1; chordku_device=abc; b=2", "chordku_device"), Some("abc"));
        assert_eq!(cookie_value("xchordku_device=abc", "chordku_device"), None);
        assert_eq!(cookie_value("", "chordku_device"), None);
    }
}
