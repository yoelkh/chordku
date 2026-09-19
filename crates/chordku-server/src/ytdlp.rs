//! yt-dlp as a managed subprocess.
//!
//! There is no pure-Rust extractor that survives YouTube's player changes, so we drive the
//! official `yt-dlp.exe`, kept under the data directory and updated from the project's
//! nightly channel (the channel its maintainers recommend). Since 2025.11 YouTube support
//! needs an external JavaScript runtime; we use Deno when present, otherwise Node >= 22.
//!
//! Safety: only http(s) URLs to public hosts are accepted, the URL is passed after `--` so
//! it can never be parsed as an option, playlists and live streams are refused, and the
//! output path is derived from the media id by us, never from remote metadata.

use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::process::Stdio;

use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use url::Url;

use crate::config::Config;

const RELEASE_API: &str = "https://api.github.com/repos/yt-dlp/yt-dlp-nightly-builds/releases/latest";
const EXE_NAME: &str = if cfg!(windows) { "yt-dlp.exe" } else { "yt-dlp" };
const USER_AGENT: &str = concat!("ChordKu/", env!("CARGO_PKG_VERSION"));

#[derive(Clone, Debug, Serialize)]
pub struct Meta {
    pub id: String,
    pub extractor: String,
    pub title: String,
    pub artist: String,
    pub duration_secs: Option<f64>,
    pub is_live: bool,
    pub thumbnail: Option<String>,
    pub webpage_url: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct Status {
    pub installed: bool,
    pub version: Option<String>,
    pub path: Option<String>,
    pub js_runtime: Option<String>,
}

static STATUS_CACHE: std::sync::Mutex<Option<(std::time::Instant, Status)>> = std::sync::Mutex::new(None);

#[derive(Clone, Debug)]
pub struct YtDlp {
    exe: PathBuf,
    js_runtime: Option<String>,
}

/// Reject anything that is not a plain http(s) URL to a public host (SSRF guard).
pub async fn validate_url(raw: &str) -> Result<Url> {
    let url = Url::parse(raw.trim()).map_err(|_| anyhow!("URL tidak valid"))?;
    if !matches!(url.scheme(), "http" | "https") {
        bail!("hanya URL http/https yang didukung");
    }
    if !url.username().is_empty() || url.password().is_some() {
        bail!("URL dengan kredensial tidak didukung");
    }
    let host = url.host_str().ok_or_else(|| anyhow!("URL tanpa host"))?.to_ascii_lowercase();
    if host == "localhost" || host.ends_with(".localhost") || host.ends_with(".local") || host.ends_with(".internal") {
        bail!("alamat lokal tidak diizinkan");
    }
    let port = url.port_or_known_default().unwrap_or(443);
    let addrs: Vec<IpAddr> = match host.trim_matches(['[', ']']).parse::<IpAddr>() {
        Ok(ip) => vec![ip],
        Err(_) => tokio::net::lookup_host((host.as_str(), port))
            .await
            .map_err(|_| anyhow!("host tidak ditemukan: {host}"))?
            .map(|a| a.ip())
            .collect(),
    };
    if addrs.is_empty() || addrs.iter().any(|ip| !is_public(*ip)) {
        bail!("alamat jaringan lokal tidak diizinkan");
    }
    Ok(url)
}

fn is_public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let o = v4.octets();
            !(v4.is_private()
                || v4.is_loopback()
                || v4.is_link_local()
                || v4.is_broadcast()
                || v4.is_unspecified()
                || v4.is_multicast()
                || o[0] == 0
                || (o[0] == 100 && (64..128).contains(&o[1])) // CGNAT 100.64/10
                || (o[0] == 192 && o[1] == 0 && o[2] == 0))
        }
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return is_public(IpAddr::V4(v4));
            }
            let first = v6.segments()[0];
            !(v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_multicast()
                || (first & 0xfe00) == 0xfc00   // unique local fc00::/7
                || (first & 0xffc0) == 0xfe80) // link local fe80::/10
        }
    }
}

fn hidden(cmd: &mut Command) -> &mut Command {
    #[cfg(windows)]
    cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW: no console flash per download
    cmd.kill_on_drop(true)
}

async fn version_of(program: &Path, arg: &str) -> Option<String> {
    let out = hidden(Command::new(program).arg(arg).stdin(Stdio::null())).output().await.ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).lines().next().unwrap_or("").trim().to_owned())
}

/// Deno (yt-dlp's default) or Node >= 22, as the value for `--js-runtimes`.
async fn find_js_runtime() -> Option<String> {
    if version_of(Path::new("deno"), "--version").await.is_some() {
        return Some("deno".into());
    }
    let node = version_of(Path::new("node"), "--version").await?;
    let major: u32 = node.trim_start_matches('v').split('.').next()?.parse().ok()?;
    (major >= 22).then(|| "node".into())
}

impl YtDlp {
    /// The managed copy if present, else one on PATH.
    pub async fn locate(cfg: &Config) -> Option<Self> {
        let managed = cfg.bin_dir().join(EXE_NAME);
        let exe = if managed.is_file() {
            managed
        } else if version_of(Path::new("yt-dlp"), "--version").await.is_some() {
            PathBuf::from("yt-dlp")
        } else {
            return None;
        };
        Some(Self { exe, js_runtime: find_js_runtime().await })
    }

    /// Cached for a minute: asking a PyInstaller exe for its version takes a second or two,
    /// which would make the settings page feel slow for no reason.
    pub async fn status(cfg: &Config) -> Status {
        if let Some((at, status)) = STATUS_CACHE.lock().expect("status cache").clone() {
            if at.elapsed() < std::time::Duration::from_secs(60) {
                return status;
            }
        }
        let status = Self::probe_status(cfg).await;
        *STATUS_CACHE.lock().expect("status cache") = Some((std::time::Instant::now(), status.clone()));
        status
    }

    async fn probe_status(cfg: &Config) -> Status {
        match Self::locate(cfg).await {
            Some(y) => Status {
                installed: true,
                version: version_of(&y.exe, "--version").await,
                path: Some(y.exe.display().to_string()),
                js_runtime: y.js_runtime,
            },
            None => Status { installed: false, version: None, path: None, js_runtime: find_js_runtime().await },
        }
    }

    /// Download (or update) the managed yt-dlp from the official nightly release and verify
    /// it against the release's SHA2-256SUMS before it replaces the current copy.
    pub async fn install(cfg: &Config) -> Result<String> {
        #[derive(Deserialize)]
        struct Asset {
            name: String,
            browser_download_url: String,
        }
        #[derive(Deserialize)]
        struct Release {
            tag_name: String,
            assets: Vec<Asset>,
        }
        let client = reqwest::Client::builder().user_agent(USER_AGENT).build()?;
        let release: Release =
            client.get(RELEASE_API).send().await?.error_for_status().context("GitHub release API")?.json().await?;
        let find = |name: &str| {
            release.assets.iter().find(|a| a.name == name).ok_or_else(|| anyhow!("rilis tidak memuat {name}"))
        };
        let sums = client.get(&find("SHA2-256SUMS")?.browser_download_url).send().await?.error_for_status()?.text().await?;
        let want = sums
            .lines()
            .filter_map(|l| l.split_once(char::is_whitespace))
            .find(|(_, name)| name.trim().trim_start_matches('*') == EXE_NAME)
            .map(|(hash, _)| hash.trim().to_ascii_lowercase())
            .ok_or_else(|| anyhow!("SHA2-256SUMS tidak memuat {EXE_NAME}"))?;
        let bytes = client.get(&find(EXE_NAME)?.browser_download_url).send().await?.error_for_status()?.bytes().await?;
        let got = format!("{:x}", Sha256::digest(&bytes));
        if got != want {
            bail!("checksum yt-dlp tidak cocok (unduhan rusak atau dimodifikasi)");
        }
        let dir = cfg.bin_dir();
        let tmp = dir.join(format!("{EXE_NAME}.new"));
        tokio::fs::write(&tmp, &bytes).await?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            tokio::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755)).await?;
        }
        *STATUS_CACHE.lock().expect("status cache") = None;
        tokio::fs::rename(&tmp, dir.join(EXE_NAME)).await.context(
            "tidak bisa mengganti yt-dlp (mungkin sedang dipakai, atau dikarantina antivirus; tambahkan folder data ChordKu ke pengecualian Windows Security)",
        )?;
        Ok(release.tag_name)
    }

    fn base(&self) -> Command {
        let mut cmd = Command::new(&self.exe);
        hidden(&mut cmd);
        cmd.args(["--no-playlist", "--no-warnings", "--ignore-config", "--no-cache-dir"]);
        if let Some(rt) = self.js_runtime.as_deref().filter(|rt| *rt != "deno") {
            cmd.args(["--js-runtimes", rt]);
        }
        cmd.stdin(Stdio::null());
        cmd
    }

    /// Metadata only (`-J`): fast, lets us reject live streams and duplicates before downloading.
    pub async fn probe(&self, url: &Url) -> Result<Meta> {
        let out = self.base().args(["-J", "--no-progress", "--"]).arg(url.as_str()).output().await.context("jalankan yt-dlp")?;
        if !out.status.success() {
            bail!("{}", friendly_error(&String::from_utf8_lossy(&out.stderr)));
        }
        parse_meta(&out.stdout)
    }

    /// Download the best audio-only stream to `dir`, named after `stem`. Returns the file.
    pub async fn download(&self, url: &Url, dir: &Path, stem: &str, mut progress: impl FnMut(f64)) -> Result<PathBuf> {
        let template = dir.join(format!("{stem}.%(ext)s"));
        let mut child = self
            .base()
            // `--print` implies quiet mode; `--progress` keeps the progress lines coming.
            .args(["-f", "bestaudio[acodec=opus]/bestaudio/best", "--newline", "--progress", "--no-part", "--no-mtime"])
            .args(["--retries", "5", "--fragment-retries", "10", "--concurrent-fragments", "4"])
            .args(["--progress-template", "download:PROG %(progress.downloaded_bytes)s %(progress.total_bytes)s %(progress.total_bytes_estimate)s"])
            .args(["--print", "after_move:FILE %(filepath)s", "-o"])
            .arg(&template)
            .arg("--")
            .arg(url.as_str())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .context("jalankan yt-dlp")?;
        let mut lines = BufReader::new(child.stdout.take().expect("piped stdout")).lines();
        let mut file = None;
        while let Some(line) = lines.next_line().await? {
            if let Some(f) = parse_progress(&line) {
                progress(f);
            } else if let Some(path) = line.strip_prefix("FILE ") {
                file = Some(PathBuf::from(path.trim()));
            }
        }
        let out = child.wait_with_output().await?;
        if !out.status.success() {
            bail!("{}", friendly_error(&String::from_utf8_lossy(&out.stderr)));
        }
        // Trust the reported path only if it is where we asked for it.
        let file = file.filter(|f| f.parent() == Some(dir) && f.is_file());
        match file {
            Some(f) => Ok(f),
            None => first_file_with_stem(dir, stem).ok_or_else(|| anyhow!("yt-dlp selesai tanpa menghasilkan file")),
        }
    }
}

fn first_file_with_stem(dir: &Path, stem: &str) -> Option<PathBuf> {
    std::fs::read_dir(dir).ok()?.flatten().map(|e| e.path()).find(|p| {
        p.is_file() && p.file_stem().and_then(|s| s.to_str()) == Some(stem)
    })
}

fn parse_meta(json: &[u8]) -> Result<Meta> {
    let v: serde_json::Value = serde_json::from_slice(json).context("keluaran yt-dlp bukan JSON")?;
    if v.get("_type").and_then(|t| t.as_str()) == Some("playlist") {
        bail!("URL ini adalah playlist; tempel URL satu lagu saja");
    }
    let s = |k: &str| v.get(k).and_then(|x| x.as_str()).map(str::to_owned);
    let live = v.get("is_live").and_then(|x| x.as_bool()).unwrap_or(false)
        || matches!(s("live_status").as_deref(), Some("is_live" | "is_upcoming"));
    Ok(Meta {
        id: s("id").ok_or_else(|| anyhow!("metadata tanpa id"))?,
        extractor: s("extractor_key").or_else(|| s("extractor")).unwrap_or_else(|| "generic".into()).to_ascii_lowercase(),
        title: s("track").or_else(|| s("title")).unwrap_or_else(|| "Tanpa judul".into()),
        artist: s("artist").or_else(|| s("creator")).or_else(|| s("uploader")).or_else(|| s("channel")).unwrap_or_default(),
        duration_secs: v.get("duration").and_then(|d| d.as_f64()),
        is_live: live,
        thumbnail: s("thumbnail").filter(|t| t.starts_with("https://")),
        webpage_url: s("webpage_url").unwrap_or_default(),
    })
}

/// `PROG <downloaded> <total> <estimate>` with `NA` for unknown values -> fraction.
fn parse_progress(line: &str) -> Option<f64> {
    let mut it = line.strip_prefix("PROG ")?.split_whitespace();
    let done: f64 = it.next()?.parse().ok()?;
    let total = it.filter_map(|t| t.parse::<f64>().ok()).find(|t| *t > 0.0)?;
    Some((done / total).clamp(0.0, 1.0))
}

/// Map yt-dlp's stderr to something a musician can act on.
fn friendly_error(stderr: &str) -> String {
    let last = stderr.lines().rev().find(|l| l.contains("ERROR")).unwrap_or(stderr.trim()).trim();
    let lower = last.to_ascii_lowercase();
    let hint = if lower.contains("private video") || lower.contains("sign in") || lower.contains("login") {
        "Video ini privat atau butuh login."
    } else if lower.contains("not available") || lower.contains("unavailable") || lower.contains("removed") {
        "Video tidak tersedia (dihapus atau diblokir di wilayah ini)."
    } else if lower.contains("unsupported url") {
        "Situs ini tidak didukung."
    } else if lower.contains("javascript") || lower.contains("js runtime") || lower.contains("nsig") || lower.contains("po token") {
        "yt-dlp perlu diperbarui atau butuh Deno/Node.js 22+. Buka Pengaturan lalu Perbarui yt-dlp."
    } else if lower.contains("timed out") || lower.contains("connection") || lower.contains("getaddrinfo") {
        "Koneksi internet bermasalah."
    } else {
        "Gagal mengambil audio."
    };
    format!("{hint} ({})", last.trim_start_matches("ERROR:").trim().chars().take(200).collect::<String>())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn rejects_local_and_odd_urls() {
        for bad in [
            "file:///C:/Windows/win.ini",
            "ftp://example.com/a.mp3",
            "http://localhost:7464/api/songs",
            "http://127.0.0.1/x",
            "http://[::1]/x",
            "http://192.168.1.10/x",
            "http://10.0.0.5/x",
            "http://169.254.169.254/latest/meta-data",
            "http://[fd00::1]/x",
            "http://[::ffff:192.168.0.1]/x",
            "https://user:pass@example.com/x",
            "--exec=calc",
            "router.local",
        ] {
            assert!(validate_url(bad).await.is_err(), "{bad} must be rejected");
        }
        assert!(validate_url("https://8.8.8.8/watch?v=1").await.is_ok());
    }

    #[test]
    fn parses_metadata_and_rejects_playlists() {
        let meta = parse_meta(
            br#"{"id":"dQw4","extractor_key":"Youtube","title":"Official Video","track":"Judul","artist":"Artis",
                "uploader":"VEVO","duration":213.0,"is_live":false,"thumbnail":"https://i.ytimg.com/x.jpg","webpage_url":"https://y/w"}"#,
        )
        .unwrap();
        assert_eq!((meta.extractor.as_str(), meta.title.as_str(), meta.artist.as_str()), ("youtube", "Judul", "Artis"));
        assert_eq!(meta.duration_secs, Some(213.0));
        assert!(parse_meta(br#"{"_type":"playlist","id":"PL1"}"#).is_err());
        let live = parse_meta(br#"{"id":"a","title":"t","live_status":"is_live"}"#).unwrap();
        assert!(live.is_live);
        let http_thumb = parse_meta(br#"{"id":"a","title":"t","thumbnail":"http://x/y.jpg"}"#).unwrap();
        assert!(http_thumb.thumbnail.is_none());
    }

    #[test]
    fn parses_progress_lines() {
        assert_eq!(parse_progress("PROG 50 200 NA"), Some(0.25));
        assert_eq!(parse_progress("PROG 50 NA 100"), Some(0.5));
        assert_eq!(parse_progress("PROG 50 NA NA"), None);
        assert_eq!(parse_progress("[download] 12%"), None);
    }

    #[test]
    fn errors_become_actionable() {
        assert!(friendly_error("WARNING: x\nERROR: [youtube] abc: Private video. Sign in").starts_with("Video ini privat"));
        assert!(friendly_error("ERROR: Unsupported URL: https://x").starts_with("Situs ini tidak didukung"));
    }
}
