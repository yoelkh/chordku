//! Audio decoding: Symphonia first, libopus for Opus packets (Symphonia has no
//! Opus decoder yet, and YouTube / MediaRecorder audio is usually WebM+Opus),
//! and an `ffmpeg` pipe as the last resort for exotic containers.

use std::fs::File;
use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};

use anyhow::{anyhow, bail, Context, Result};
use symphonia::core::audio::sample::Sample;
use symphonia::core::codecs::audio::well_known::CODEC_ID_OPUS;
use symphonia::core::codecs::audio::AudioDecoderOptions;
use symphonia::core::errors::Error as SymError;
use symphonia::core::formats::probe::Hint;
use symphonia::core::formats::{FormatOptions, FormatReader, TrackType};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;

use crate::audio::Audio;

/// Hard cap so a hostile or endless stream cannot exhaust memory (3 h of 48 kHz stereo).
const MAX_SAMPLES: usize = 3 * 3600 * 48_000 * 2;
const OPUS_RATE: u32 = 48_000;
/// 120 ms, the longest Opus frame, at 48 kHz.
const OPUS_MAX_FRAME: usize = 5760;

/// Decode any supported file to interleaved f32 at its native rate.
pub fn decode_file(path: &Path) -> Result<Audio> {
    match decode_native(path) {
        Ok(a) if a.frames() > 0 => Ok(a),
        Ok(_) => decode_ffmpeg(path).context("native decoder produced no audio"),
        Err(native) => decode_ffmpeg(path).map_err(|ff| {
            anyhow!("cannot decode {}: {native:#}; ffmpeg fallback: {ff:#}", path.display())
        }),
    }
}

fn decode_native(path: &Path) -> Result<Audio> {
    let file = File::open(path).with_context(|| format!("open {}", path.display()))?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }
    let mut format = symphonia::default::get_probe()
        .probe(&hint, mss, FormatOptions::default(), MetadataOptions::default())
        .context("unrecognised container")?;

    let track = format.default_track(TrackType::Audio).ok_or_else(|| anyhow!("no audio track"))?;
    let track_id = track.id;
    let params = track
        .codec_params
        .as_ref()
        .and_then(|p| p.audio())
        .ok_or_else(|| anyhow!("track has no audio parameters"))?
        .clone();

    if params.codec == CODEC_ID_OPUS {
        let channels = params.channels.as_ref().map(|c| c.count()).unwrap_or(2).clamp(1, 2);
        let pre_skip = opus_pre_skip(params.extra_data.as_deref());
        return decode_opus_packets(format.as_mut(), track_id, channels, pre_skip);
    }

    let mut decoder = symphonia::default::get_codecs()
        .make_audio_decoder(&params, &AudioDecoderOptions::default())
        .context("unsupported codec")?;

    let mut out = Audio::default();
    let mut scratch: Vec<f32> = Vec::new();
    loop {
        let packet = match format.next_packet() {
            Ok(Some(p)) => p,
            Ok(None) => break,
            Err(SymError::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(e) => {
                if out.samples.is_empty() {
                    return Err(e.into());
                }
                break; // keep what we have from a truncated file
            }
        };
        if packet.track_id != track_id {
            continue;
        }
        match decoder.decode(&packet) {
            Ok(buf) => {
                if out.channels == 0 {
                    out.sample_rate = buf.spec().rate();
                    out.channels = buf.spec().channels().count();
                }
                scratch.resize(buf.samples_interleaved(), f32::MID);
                buf.copy_to_slice_interleaved(&mut scratch);
                out.samples.extend_from_slice(&scratch);
                if out.samples.len() > MAX_SAMPLES {
                    bail!("audio is longer than the 3 hour safety limit");
                }
            }
            Err(SymError::DecodeError(_)) => continue, // skip a corrupt packet
            Err(e) => return Err(e.into()),
        }
    }
    Ok(out)
}

/// `OpusHead` layout: magic(8) version(1) channels(1) pre_skip(u16 LE) ...
fn opus_pre_skip(extra: Option<&[u8]>) -> usize {
    match extra {
        Some(d) if d.len() >= 12 && &d[..8] == b"OpusHead" => {
            u16::from_le_bytes([d[10], d[11]]) as usize
        }
        _ => 312, // default libopus encoder delay at 48 kHz
    }
}

fn decode_opus_packets(
    format: &mut dyn FormatReader,
    track_id: u32,
    channels: usize,
    pre_skip: usize,
) -> Result<Audio> {
    let ch = if channels == 1 { opus::Channels::Mono } else { opus::Channels::Stereo };
    let mut dec = opus::Decoder::new(OPUS_RATE, ch).context("create opus decoder")?;
    let mut out = Audio { sample_rate: OPUS_RATE, channels, samples: Vec::new() };
    let mut frame = vec![0.0f32; OPUS_MAX_FRAME * channels];
    while let Ok(Some(packet)) = format.next_packet() {
        if packet.track_id != track_id || packet.data.is_empty() {
            continue;
        }
        if let Ok(n) = dec.decode_float(&packet.data, &mut frame, false) {
            out.samples.extend_from_slice(&frame[..n * channels]);
        }
        if out.samples.len() > MAX_SAMPLES {
            bail!("audio is longer than the 3 hour safety limit");
        }
    }
    let skip = (pre_skip * channels).min(out.samples.len());
    out.samples.drain(..skip);
    Ok(out)
}

/// Last resort: let ffmpeg (if installed) hand us 44.1 kHz stereo f32.
fn decode_ffmpeg(path: &Path) -> Result<Audio> {
    let mut cmd = Command::new("ffmpeg");
    cmd.args(["-v", "error", "-nostdin", "-i"])
        .arg(path)
        .args(["-vn", "-f", "f32le", "-acodec", "pcm_f32le", "-ac", "2", "-ar", "44100", "-"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let mut child = cmd.spawn().context("ffmpeg is not installed")?;
    let mut bytes = Vec::new();
    child
        .stdout
        .take()
        .expect("piped stdout")
        .take((MAX_SAMPLES * 4) as u64)
        .read_to_end(&mut bytes)?;
    let status = child.wait()?;
    if bytes.len() < 8 {
        bail!("ffmpeg produced no audio ({status})");
    }
    let samples =
        bytes.chunks_exact(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect();
    Ok(Audio { sample_rate: 44_100, channels: 2, samples })
}
