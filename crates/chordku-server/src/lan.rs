//! Opt-in access from a phone on the same Wi-Fi.
//!
//! The laptop keeps talking plain HTTP to 127.0.0.1 (already a secure context). LAN mode
//! adds a second listener on `0.0.0.0:<port + 1>` that is HTTPS only, because a browser
//! gives a page the microphone only over HTTPS.
//!
//! * TLS: a private CA is generated once; a leaf certificate for the machine's current LAN
//!   addresses is issued from it and re-issued when the address changes (new Wi-Fi, DHCP).
//!   Installing the CA on the phone is optional: without it the browser shows a warning
//!   page once, after which everything including recording works.
//! * Pairing: every LAN request needs a device cookie. It is obtained by opening a one-time
//!   link (shown as a QR code on the laptop) within ten minutes. Only a hash of the device
//!   token is stored.
//! * Only private-range peers are served, and administrative endpoints stay loopback-only.

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{anyhow, Context, Result};
use axum_server::tls_rustls::RustlsConfig;
use rand::RngCore;
use rcgen::{
    BasicConstraints, CertificateParams, DistinguishedName, DnType, ExtendedKeyUsagePurpose, IsCa, Issuer, KeyPair,
    KeyUsagePurpose, SanType,
};
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::config::Config;

pub const COOKIE: &str = "chordku_device";
const PAIR_TTL: Duration = Duration::from_secs(10 * 60);
const SETTING_ENABLED: &str = "lan_enabled";

#[derive(Clone, Debug, Serialize)]
pub struct LanInfo {
    pub enabled: bool,
    pub running: bool,
    pub port: u16,
    pub addresses: Vec<String>,
    /// `https://<ip>:<port>/#/pair/<code>`, present while a pairing code is active.
    pub pair_url: Option<String>,
    pub pair_qr_svg: Option<String>,
    pub pair_expires_secs: Option<u64>,
    pub error: Option<String>,
}

#[derive(Default)]
struct Inner {
    running: bool,
    addresses: Vec<Ipv4Addr>,
    error: Option<String>,
    pair_codes: HashMap<String, Instant>,
    handle: Option<axum_server::Handle<SocketAddr>>,
}

#[derive(Clone)]
pub struct Lan {
    cfg: Arc<Config>,
    inner: Arc<Mutex<Inner>>,
}

/// Private IPv4 addresses of this machine. The UDP "connect" sends nothing: it only asks
/// the OS which local address would route towards the target.
pub fn local_ipv4s() -> Vec<Ipv4Addr> {
    let mut out = Vec::new();
    for target in ["192.168.0.1:9", "10.0.0.1:9", "172.16.0.1:9", "8.8.8.8:9"] {
        let ip = UdpSocket::bind("0.0.0.0:0").and_then(|s| s.connect(target).and_then(|_| s.local_addr())).map(|a| a.ip());
        if let Ok(IpAddr::V4(v4)) = ip {
            if v4.is_private() && !out.contains(&v4) {
                out.push(v4);
            }
        }
    }
    out
}

pub fn is_private_peer(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => v4.is_private() || v4.is_loopback() || v4.is_link_local(),
        IpAddr::V6(v6) => v6.to_ipv4_mapped().map(|v4| is_private_peer(IpAddr::V4(v4))).unwrap_or_else(|| {
            v6.is_loopback() || (v6.segments()[0] & 0xfe00) == 0xfc00 || (v6.segments()[0] & 0xffc0) == 0xfe80
        }),
    }
}

pub fn hash_token(token: &str) -> String {
    format!("{:x}", Sha256::digest(token.as_bytes()))
}

fn random_hex(bytes: usize) -> String {
    let mut buf = vec![0u8; bytes];
    rand::rng().fill_bytes(&mut buf);
    buf.iter().map(|b| format!("{b:02x}")).collect()
}

fn ca_params() -> CertificateParams {
    let mut params = CertificateParams::default();
    let mut dn = DistinguishedName::new();
    dn.push(DnType::CommonName, "ChordKu Local CA");
    dn.push(DnType::OrganizationName, "ChordKu");
    params.distinguished_name = dn;
    params.is_ca = IsCa::Ca(BasicConstraints::Constrained(0));
    params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
    params.not_before = time::OffsetDateTime::now_utc() - time::Duration::days(1);
    params.not_after = time::OffsetDateTime::now_utc() + time::Duration::days(3650);
    params
}

impl Lan {
    pub fn new(cfg: Arc<Config>) -> Self {
        Self { cfg, inner: Arc::default() }
    }

    fn tls_dir(&self) -> PathBuf {
        self.cfg.data_dir.join("tls")
    }

    pub fn port(&self) -> u16 {
        self.cfg.port.wrapping_add(1)
    }

    pub fn ca_cert_path(&self) -> PathBuf {
        self.tls_dir().join("chordku-ca.crt")
    }

    /// Load or create the CA. The key never leaves the data directory and is never
    /// regenerated, so a phone that trusted it keeps trusting it.
    fn ca(&self) -> Result<(KeyPair, String)> {
        let dir = self.tls_dir();
        std::fs::create_dir_all(&dir)?;
        let key_path = dir.join("chordku-ca.key");
        let cert_path = self.ca_cert_path();
        if key_path.is_file() && cert_path.is_file() {
            let key = KeyPair::from_pem(&std::fs::read_to_string(&key_path)?).context("CA key")?;
            return Ok((key, std::fs::read_to_string(&cert_path)?));
        }
        let key = KeyPair::generate()?;
        let cert = ca_params().self_signed(&key)?.pem();
        std::fs::write(&key_path, key.serialize_pem())?;
        std::fs::write(&cert_path, &cert)?;
        Ok((key, cert))
    }

    /// Leaf certificate + key (PEM) for the given addresses, chained to the CA.
    fn issue_leaf(&self, ips: &[Ipv4Addr]) -> Result<(Vec<u8>, Vec<u8>)> {
        let (ca_key, ca_pem) = self.ca()?;
        let issuer = Issuer::new(ca_params(), ca_key);
        let mut params = CertificateParams::new(vec!["localhost".to_string()])?;
        let mut dn = DistinguishedName::new();
        dn.push(DnType::CommonName, "ChordKu");
        params.distinguished_name = dn;
        params.subject_alt_names.push(SanType::IpAddress(IpAddr::V4(Ipv4Addr::LOCALHOST)));
        for ip in ips {
            params.subject_alt_names.push(SanType::IpAddress(IpAddr::V4(*ip)));
        }
        params.key_usages = vec![KeyUsagePurpose::DigitalSignature, KeyUsagePurpose::KeyEncipherment];
        params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
        params.not_before = time::OffsetDateTime::now_utc() - time::Duration::days(1);
        // Browsers reject server certificates valid for more than 398 days.
        params.not_after = time::OffsetDateTime::now_utc() + time::Duration::days(365);
        let leaf_key = KeyPair::generate()?;
        let leaf = params.signed_by(&leaf_key, &issuer)?;
        let chain = format!("{}{}", leaf.pem(), ca_pem);
        Ok((chain.into_bytes(), leaf_key.serialize_pem().into_bytes()))
    }

    pub fn is_enabled(&self, db: &crate::db::Db) -> bool {
        db.setting(SETTING_ENABLED).ok().flatten().as_deref() == Some("1")
    }

    /// Hosts the LAN listener answers to.
    pub fn allowed_host(&self, name: &str) -> bool {
        let inner = self.inner.lock().expect("lan mutex");
        name == "localhost" || name == "127.0.0.1" || inner.addresses.iter().any(|ip| ip.to_string() == name)
    }

    /// Start (or restart) the HTTPS listener with `router`. Re-issues the certificate
    /// whenever the machine's LAN addresses change.
    pub async fn start(&self, router: axum::Router) -> Result<()> {
        self.stop();
        let ips = local_ipv4s();
        if ips.is_empty() {
            let msg = "Tidak ada alamat jaringan lokal. Sambungkan laptop ke Wi-Fi dulu.";
            self.inner.lock().expect("lan mutex").error = Some(msg.into());
            return Err(anyhow!(msg));
        }
        let (cert, key) = self.issue_leaf(&ips)?;
        let tls = RustlsConfig::from_pem(cert, key).await.context("TLS config")?;
        let handle = axum_server::Handle::new();
        let addr = SocketAddr::from((Ipv4Addr::UNSPECIFIED, self.port()));
        {
            let mut inner = self.inner.lock().expect("lan mutex");
            inner.addresses = ips.clone();
            inner.handle = Some(handle.clone());
            inner.running = true;
            inner.error = None;
        }

        let me = self.clone();
        let serve_handle = handle.clone();
        tokio::spawn(async move {
            let server = axum_server::bind_rustls(addr, tls.clone())
                .handle(serve_handle)
                .serve(router.into_make_service_with_connect_info::<SocketAddr>());
            // Watch for address changes while serving.
            let watcher = {
                let me = me.clone();
                async move {
                    let mut current = ips;
                    loop {
                        tokio::time::sleep(Duration::from_secs(30)).await;
                        let now = local_ipv4s();
                        if !now.is_empty() && now != current {
                            match me.issue_leaf(&now) {
                                Ok((cert, key)) => {
                                    if tls.reload_from_pem(cert, key).await.is_ok() {
                                        tracing::info!("LAN address changed to {now:?}; certificate re-issued");
                                        me.inner.lock().expect("lan mutex").addresses = now.clone();
                                        current = now;
                                    }
                                }
                                Err(e) => tracing::warn!("cannot re-issue LAN certificate: {e:#}"),
                            }
                        }
                    }
                }
            };
            let result = tokio::select! {
                r = server => r,
                _ = watcher => Ok(()),
            };
            let mut inner = me.inner.lock().expect("lan mutex");
            inner.running = false;
            if let Err(e) = result {
                tracing::warn!("LAN listener stopped: {e}");
                inner.error = Some(format!("Port {} tidak bisa dibuka: {e}", addr.port()));
            }
        });
        tracing::info!("mode LAN aktif di https://{}:{}", self.inner.lock().expect("lan mutex").addresses[0], self.port());
        Ok(())
    }

    pub fn stop(&self) {
        let mut inner = self.inner.lock().expect("lan mutex");
        if let Some(handle) = inner.handle.take() {
            handle.graceful_shutdown(Some(Duration::from_secs(1)));
        }
        inner.running = false;
        inner.pair_codes.clear();
    }

    /// New one-time pairing code (invalidates older ones).
    pub fn new_pair_code(&self) -> String {
        let code = random_hex(16);
        let mut inner = self.inner.lock().expect("lan mutex");
        inner.pair_codes.clear();
        inner.pair_codes.insert(code.clone(), Instant::now() + PAIR_TTL);
        code
    }

    /// Consume a pairing code. Returns a fresh device token on success.
    pub fn redeem(&self, code: &str) -> Option<String> {
        let mut inner = self.inner.lock().expect("lan mutex");
        inner.pair_codes.retain(|_, expiry| *expiry > Instant::now());
        inner.pair_codes.remove(code).map(|_| random_hex(32))
    }

    pub fn info(&self, db: &crate::db::Db) -> LanInfo {
        let mut inner = self.inner.lock().expect("lan mutex");
        inner.pair_codes.retain(|_, expiry| *expiry > Instant::now());
        let port = self.port();
        let pair = inner.pair_codes.iter().next().and_then(|(code, expiry)| {
            inner.addresses.first().map(|ip| (format!("https://{ip}:{port}/#/pair/{code}"), expiry.saturating_duration_since(Instant::now()).as_secs()))
        });
        let qr = pair.as_ref().and_then(|(url, _)| {
            qrcode::QrCode::new(url.as_bytes()).ok().map(|code| {
                code.render::<qrcode::render::svg::Color>().min_dimensions(220, 220).quiet_zone(true).build()
            })
        });
        LanInfo {
            enabled: db.setting(SETTING_ENABLED).ok().flatten().as_deref() == Some("1"),
            running: inner.running,
            port,
            addresses: inner.addresses.iter().map(|ip| format!("https://{ip}:{port}")).collect(),
            pair_url: pair.as_ref().map(|(u, _)| u.clone()),
            pair_expires_secs: pair.as_ref().map(|(_, s)| *s),
            pair_qr_svg: qr,
            error: inner.error.clone(),
        }
    }

    pub fn set_enabled(&self, db: &crate::db::Db, enabled: bool) -> Result<()> {
        db.set_setting(SETTING_ENABLED, if enabled { "1" } else { "0" })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lan() -> (Lan, PathBuf) {
        let dir = std::env::temp_dir().join(format!("chordku-lan-test-{}-{}", std::process::id(), random_hex(4)));
        let cfg = Config {
            data_dir: dir.clone(),
            port: 7464,
            bind: IpAddr::V4(Ipv4Addr::LOCALHOST),
            max_duration_secs: 1200,
            max_upload_bytes: 1,
            threads: 0,
            min_free_ram: 0,
            model_search: vec![],
        };
        (Lan::new(Arc::new(cfg)), dir)
    }

    #[test]
    fn ca_is_stable_and_leaf_chains_to_it() {
        let (lan, dir) = lan();
        let (_, ca1) = lan.ca().unwrap();
        let (_, ca2) = lan.ca().unwrap();
        assert_eq!(ca1, ca2, "the CA must not be regenerated");
        let (chain, key) = lan.issue_leaf(&[Ipv4Addr::new(192, 168, 1, 23)]).unwrap();
        let chain = String::from_utf8(chain).unwrap();
        assert_eq!(chain.matches("BEGIN CERTIFICATE").count(), 2);
        assert!(chain.ends_with(&ca1));
        assert!(String::from_utf8(key).unwrap().contains("PRIVATE KEY"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn pairing_codes_are_single_use() {
        let (lan, dir) = lan();
        let code = lan.new_pair_code();
        assert!(lan.redeem("wrong").is_none());
        let token = lan.redeem(&code).expect("valid code");
        assert_eq!(token.len(), 64);
        assert!(lan.redeem(&code).is_none(), "second use must fail");
        assert_ne!(hash_token(&token), token);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn peer_ranges() {
        assert!(is_private_peer("192.168.1.5".parse().unwrap()));
        assert!(is_private_peer("10.1.2.3".parse().unwrap()));
        assert!(is_private_peer("::ffff:192.168.1.5".parse().unwrap()));
        assert!(!is_private_peer("8.8.8.8".parse().unwrap()));
        assert!(!is_private_peer("2001:4860:4860::8888".parse().unwrap()));
    }
}
