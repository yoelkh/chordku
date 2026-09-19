//! Streaming Ogg Opus writer for stems.
//!
//! Stems leave the separator at 44.1 kHz in ~6 s blocks while Opus only runs at 48 kHz, so
//! each writer owns a small streaming polyphase resampler (exact ratio 160/147). Nothing is
//! buffered beyond one 20 ms frame: a six minute, four stem separation never holds more
//! than the block in flight.
//!
//! Four stereo stems of a four minute song at 96 kbit/s are ~11 MB, against ~160 MB of
//! FLAC, which is the difference between a phone starting playback in a second or in
//! fifteen. The browser decodes each file to PCM once, so the codec cannot cause drift
//! between stems; identical frame sizes, pre-skip and lengths keep them sample aligned.

use std::io::Write;

use anyhow::{Context, Result};
use ogg::writing::{PacketWriteEndInfo, PacketWriter};

const OPUS_RATE: u32 = 48_000;
const FRAME: usize = 960; // 20 ms
const MAX_PACKET: usize = 4000;

pub struct OggOpusWriter<W: Write> {
    ogg: PacketWriter<'static, W>,
    encoder: opus::Encoder,
    channels: usize,
    serial: u32,
    pre_skip: usize,
    resamplers: Vec<Polyphase>,
    /// Interleaved 48 kHz samples waiting for a full frame.
    pending: Vec<f32>,
    scratch: Vec<Vec<f32>>,
    /// 48 kHz frames (per channel) handed to the encoder / received from the caller.
    encoded: u64,
    received: u64,
    packet: Vec<u8>,
}

impl<W: Write> OggOpusWriter<W> {
    /// `channels` is 1 or 2; input passed to [`write`](Self::write) is planar 44.1 kHz.
    pub fn new(out: W, channels: usize, bitrate: i32, serial: u32) -> Result<Self> {
        anyhow::ensure!(channels == 1 || channels == 2, "opus stems are mono or stereo");
        let layout = if channels == 1 { opus::Channels::Mono } else { opus::Channels::Stereo };
        let mut encoder = opus::Encoder::new(OPUS_RATE, layout, opus::Application::Audio).context("opus encoder")?;
        encoder.set_bitrate(opus::Bitrate::Bits(bitrate))?;
        let pre_skip = encoder.get_lookahead()?.max(0) as usize;

        let mut ogg = PacketWriter::new(out);
        let mut head = Vec::with_capacity(19);
        head.extend_from_slice(b"OpusHead");
        head.push(1); // version
        head.push(channels as u8);
        head.extend_from_slice(&(pre_skip as u16).to_le_bytes());
        head.extend_from_slice(&44_100u32.to_le_bytes()); // original input rate, informational
        head.extend_from_slice(&0i16.to_le_bytes()); // output gain
        head.push(0); // channel mapping family 0
        ogg.write_packet(head, serial, PacketWriteEndInfo::EndPage, 0)?;

        let vendor = b"ChordKu";
        let mut tags = Vec::new();
        tags.extend_from_slice(b"OpusTags");
        tags.extend_from_slice(&(vendor.len() as u32).to_le_bytes());
        tags.extend_from_slice(vendor);
        tags.extend_from_slice(&0u32.to_le_bytes());
        ogg.write_packet(tags, serial, PacketWriteEndInfo::EndPage, 0)?;

        Ok(Self {
            ogg,
            encoder,
            channels,
            serial,
            pre_skip,
            resamplers: (0..channels).map(|_| Polyphase::new()).collect(),
            pending: Vec::with_capacity(FRAME * channels * 2),
            scratch: vec![Vec::new(); channels],
            encoded: 0,
            received: 0,
            packet: vec![0u8; MAX_PACKET],
        })
    }

    /// Append planar 44.1 kHz audio (`planar.len()` must equal the channel count).
    pub fn write(&mut self, planar: &[&[f32]]) -> Result<()> {
        anyhow::ensure!(planar.len() == self.channels, "channel count mismatch");
        for (c, input) in planar.iter().enumerate() {
            self.scratch[c].clear();
            self.resamplers[c].process(input, &mut self.scratch[c]);
        }
        self.queue_scratch();
        self.drain(false)
    }

    fn queue_scratch(&mut self) {
        let n = self.scratch.iter().map(Vec::len).min().unwrap_or(0);
        for i in 0..n {
            for c in 0..self.channels {
                self.pending.push(self.scratch[c][i]);
            }
        }
        self.received += n as u64;
    }

    fn drain(&mut self, finish: bool) -> Result<()> {
        let step = FRAME * self.channels;
        if finish {
            // Pad so the decoder can emit every real sample after discarding `pre_skip`.
            let need = self.received + self.pre_skip as u64;
            let have = self.encoded + (self.pending.len() / self.channels) as u64;
            let frames = need.saturating_sub(have).div_ceil(FRAME as u64).max(1);
            let target = self.pending.len().div_ceil(step) * step + (frames as usize - 1) * step;
            self.pending.resize(target.max(step), 0.0);
        }
        let mut at = 0;
        while self.pending.len() - at >= step {
            let last = finish && self.pending.len() - at == step;
            let len = self.encoder.encode_float(&self.pending[at..at + step], &mut self.packet)?;
            self.encoded += FRAME as u64;
            // The final granule position trims the padding: real samples plus pre-skip.
            let granule = if last { self.received + self.pre_skip as u64 } else { self.encoded };
            let end = if last { PacketWriteEndInfo::EndStream } else { PacketWriteEndInfo::NormalPacket };
            self.ogg.write_packet(self.packet[..len].to_vec(), self.serial, end, granule)?;
            at += step;
        }
        self.pending.drain(..at);
        Ok(())
    }

    /// Flush the resampler and encoder and close the stream.
    pub fn finish(mut self) -> Result<W> {
        for c in 0..self.channels {
            self.scratch[c].clear();
            let (rs, out) = (&mut self.resamplers[c], &mut self.scratch[c]);
            rs.flush(out);
        }
        self.queue_scratch();
        self.drain(true)?;
        Ok(self.ogg.into_inner())
    }
}

/// Streaming 44.1 -> 48 kHz polyphase FIR resampler (L = 160, M = 147).
///
/// A Kaiser-windowed sinc (beta 9, 32 taps per phase) puts the first alias image more than
/// 80 dB down, far below anything a 96 kbit/s Opus stream preserves.
pub struct Polyphase {
    /// `[L][TAPS]` coefficients.
    phases: Vec<[f32; TAPS]>,
    history: Vec<f32>,
    /// Position of the next output sample in units of 1/L input samples.
    phase_acc: usize,
    started: bool,
}

const L: usize = 160;
const M: usize = 147;
const TAPS: usize = 32;

impl Default for Polyphase {
    fn default() -> Self {
        Self::new()
    }
}

impl Polyphase {
    pub fn new() -> Self {
        let beta = 9.0f64;
        let cutoff = 0.5 * 0.94; // of the input Nyquist; leaves a transition band up to 22.05 kHz
        let i0_beta = bessel_i0(beta);
        let centre = (TAPS as f64 - 1.0) / 2.0;
        let mut phases = vec![[0f32; TAPS]; L];
        for (p, taps) in phases.iter_mut().enumerate() {
            let frac = p as f64 / L as f64;
            let mut sum = 0f64;
            let mut row = [0f64; TAPS];
            for (k, slot) in row.iter_mut().enumerate() {
                // Output sample sits `frac` after input sample `centre`'s integer part.
                let x = k as f64 - centre.floor() - frac;
                let sinc = if x.abs() < 1e-12 {
                    2.0 * cutoff
                } else {
                    (2.0 * std::f64::consts::PI * cutoff * x).sin() / (std::f64::consts::PI * x)
                };
                // Kaiser window centred on the output instant, like the sinc.
                let pos = x / (TAPS as f64 / 2.0);
                let w = if pos.abs() >= 1.0 { 0.0 } else { bessel_i0(beta * (1.0 - pos * pos).sqrt()) / i0_beta };
                *slot = sinc * w;
                sum += *slot;
            }
            for (t, v) in taps.iter_mut().zip(row) {
                *t = (v / sum) as f32; // unity DC gain per phase
            }
        }
        Self { phases, history: Vec::new(), phase_acc: 0, started: false }
    }

    /// Feed input, append every output sample that can be computed so far.
    pub fn process(&mut self, input: &[f32], out: &mut Vec<f32>) {
        if !self.started {
            // Prime with zeros so the first output aligns with the first input sample.
            self.history.extend(std::iter::repeat(0.0).take(TAPS / 2 - 1));
            self.started = true;
        }
        self.history.extend_from_slice(input);
        self.run(out);
    }

    /// Emit the tail by feeding half a filter of silence.
    pub fn flush(&mut self, out: &mut Vec<f32>) {
        if self.started {
            self.history.extend(std::iter::repeat(0.0).take(TAPS / 2 + 1));
            self.run(out);
        }
    }

    fn run(&mut self, out: &mut Vec<f32>) {
        let mut base = self.phase_acc / L;
        while base + TAPS <= self.history.len() {
            let taps = &self.phases[self.phase_acc % L];
            let window = &self.history[base..base + TAPS];
            out.push(taps.iter().zip(window).map(|(a, b)| a * b).sum());
            self.phase_acc += M;
            base = self.phase_acc / L;
        }
        // Drop consumed input, keeping the phase accumulator relative to the new start.
        let consumed = base.min(self.history.len());
        self.history.drain(..consumed);
        self.phase_acc -= consumed * L;
    }
}

fn bessel_i0(x: f64) -> f64 {
    let (mut sum, mut term, mut k) = (1.0, 1.0, 1.0);
    while term > 1e-12 * sum {
        term *= (x / (2.0 * k)).powi(2);
        sum += term;
        k += 1.0;
    }
    sum
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(freq: f32, n: usize) -> Vec<f32> {
        (0..n).map(|i| (2.0 * std::f32::consts::PI * freq * i as f32 / 44_100.0).sin() * 0.5).collect()
    }

    #[test]
    fn polyphase_keeps_pitch_level_and_length() {
        let x = tone(1000.0, 44_100);
        let mut rs = Polyphase::new();
        let mut y = Vec::new();
        for block in x.chunks(3001) {
            rs.process(block, &mut y); // odd block size exercises the streaming state
        }
        rs.flush(&mut y);
        assert!((y.len() as i64 - 48_000).abs() <= 2, "len {}", y.len());
        let mid = &y[4800..43_200];
        let peak = mid.iter().fold(0f32, |m, v| m.max(v.abs()));
        assert!((peak - 0.5).abs() < 0.005, "peak {peak}");
        let zc = mid.windows(2).filter(|w| (w[0] < 0.0) != (w[1] < 0.0)).count();
        assert!((zc as i64 - 1600).abs() <= 2, "zero crossings {zc}"); // 0.8 s of 1 kHz
    }

    #[test]
    fn streaming_equals_one_shot() {
        let x = tone(3000.0, 20_000);
        let (mut a, mut b) = (Vec::new(), Vec::new());
        let mut one = Polyphase::new();
        one.process(&x, &mut a);
        one.flush(&mut a);
        let mut many = Polyphase::new();
        for block in x.chunks(777) {
            many.process(block, &mut b);
        }
        many.flush(&mut b);
        assert_eq!(a.len(), b.len());
        assert!(a.iter().zip(&b).all(|(p, q)| (p - q).abs() < 1e-6));
    }

    #[test]
    fn ogg_opus_round_trips_through_our_decoder() {
        let secs = 2usize;
        let left = tone(440.0, 44_100 * secs);
        let right = tone(660.0, 44_100 * secs);
        let mut w = OggOpusWriter::new(Vec::new(), 2, 96_000, 1).unwrap();
        for (l, r) in left.chunks(10_000).zip(right.chunks(10_000)) {
            w.write(&[l, r]).unwrap();
        }
        let bytes = w.finish().unwrap();
        assert!(bytes.len() > 10_000 && bytes.len() < 60_000, "{} bytes", bytes.len());

        let path = std::env::temp_dir().join(format!("chordku-opus-test-{}.opus", std::process::id()));
        std::fs::write(&path, &bytes).unwrap();
        let audio = crate::decode::decode_file(&path).unwrap();
        let _ = std::fs::remove_file(&path);
        assert_eq!((audio.sample_rate, audio.channels), (48_000, 2));
        // The decoder does not apply the end trim, so allow up to a frame of padding.
        let want = 48_000 * secs;
        assert!(audio.frames() >= want && audio.frames() <= want + 2 * FRAME, "frames {}", audio.frames());
        let [l, _] = audio.to_stereo_planar();
        let zc = l[9600..86_400].windows(2).filter(|w| (w[0] < 0.0) != (w[1] < 0.0)).count();
        assert!((zc as i64 - 1408).abs() <= 3, "zero crossings {zc}"); // 1.6 s of 440 Hz
    }
}
