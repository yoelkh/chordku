//! Drum transcription from the separated drum stem: when kick, snare and hi-hat hits happen.
//!
//! The stem is clean enough (htdemucs) that the classic band-wise method works: a short-time
//! spectrum, the energy of four bands, and the half-wave rectified rise of each band's
//! log-energy as its onset signal (spectral flux, Bello et al. 2005). A hit is a peak of the
//! combined flux; which drum it was follows from which bands rose:
//!
//!   kick    35–110 Hz        the fundamental thump
//!   snare   130–320 Hz body  plus a broadband 1.2–4.5 kHz burst (the wires)
//!   hi-hat  6–15 kHz         and almost nothing below 1 kHz
//!
//! Strengths are 0–100 per drum, several can be set on one hit (kick + hi-hat together is
//! the most common thing in pop). Placing the hits on the bar grid is left to the client, so
//! a corrected downbeat or tempo re-quantises the tab without touching audio.

use std::sync::Arc;

use realfft::{RealFftPlanner, RealToComplex};
use serde::{Deserialize, Serialize};

pub const RATE: u32 = 44_100;
const N_FFT: usize = 2048;
const HOP: usize = 441; // 10 ms
const BINS: usize = N_FFT / 2 + 1;

/// Frequency bands, Hz.
const KICK: (f32, f32) = (35.0, 110.0);
const SNARE: (f32, f32) = (130.0, 320.0);
const WIRES: (f32, f32) = (1200.0, 4500.0);
const HAT: (f32, f32) = (6000.0, 15000.0);

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct DrumOnset {
    pub t_ms: u32,
    /// 0 = not this drum, 100 = as hard as this recording gets.
    pub kick: u8,
    pub snare: u8,
    pub hat: u8,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct DrumEvents {
    pub version: u8,
    pub onsets: Vec<DrumOnset>,
}

pub const VERSION: u8 = 1;

struct Bands {
    kick: Vec<f32>,
    snare: Vec<f32>,
    wires: Vec<f32>,
    hat: Vec<f32>,
}

fn bin_range(hz: (f32, f32)) -> std::ops::Range<usize> {
    let lo = (hz.0 * N_FFT as f32 / RATE as f32).round() as usize;
    let hi = (hz.1 * N_FFT as f32 / RATE as f32).round() as usize;
    lo.min(BINS - 1)..(hi + 1).min(BINS)
}

/// Band powers per 10 ms frame of a mono 44.1 kHz signal.
fn band_powers(x: &[f32]) -> Bands {
    let fft: Arc<dyn RealToComplex<f32>> = RealFftPlanner::<f32>::new().plan_fft_forward(N_FFT);
    let window: Vec<f32> = (0..N_FFT).map(|i| 0.5 - 0.5 * (2.0 * std::f32::consts::PI * i as f32 / N_FFT as f32).cos()).collect();
    let frames = x.len().div_ceil(HOP);
    let mut bands = Bands { kick: vec![0.0; frames], snare: vec![0.0; frames], wires: vec![0.0; frames], hat: vec![0.0; frames] };
    let mut input = fft.make_input_vec();
    let mut output = fft.make_output_vec();
    let (rk, rs, rw, rh) = (bin_range(KICK), bin_range(SNARE), bin_range(WIRES), bin_range(HAT));
    let half = N_FFT / 2;
    for f in 0..frames {
        let center = f * HOP;
        for (i, v) in input.iter_mut().enumerate() {
            let n = center as isize + i as isize - half as isize;
            *v = if n >= 0 && (n as usize) < x.len() { x[n as usize] * window[i] } else { 0.0 };
        }
        if fft.process(&mut input, &mut output).is_err() {
            break;
        }
        let power = |r: &std::ops::Range<usize>| output[r.clone()].iter().map(|c| c.norm_sqr()).sum::<f32>();
        bands.kick[f] = power(&rk);
        bands.snare[f] = power(&rs);
        bands.wires[f] = power(&rw);
        bands.hat[f] = power(&rh);
    }
    bands
}

fn percentile(values: &[f32], p: f32) -> f32 {
    let mut v: Vec<f32> = values.iter().copied().filter(|x| x.is_finite()).collect();
    if v.is_empty() {
        return 0.0;
    }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    v[((v.len() - 1) as f32 * p).round() as usize]
}

/// Spectral flux of a band: rise of its log energy, scaled so the strong hits of this
/// recording sit near 1. Energy is normalised by the band's own loud level first, so a quiet
/// hi-hat counts as much as a loud kick within its own band. The scale is the 98th
/// percentile over *all* frames: hits occupy a few percent of a drum track, so that is a
/// clear hit, and the many tiny rises between hits stay small.
fn flux(power: &[f32]) -> Vec<f32> {
    let loud = percentile(power, 0.97).max(1e-12);
    let log: Vec<f32> = power.iter().map(|p| (1.0 + 30.0 * p / loud).ln()).collect();
    let mut out = vec![0.0; log.len()];
    for t in 1..log.len() {
        // Compare with the quieter of the two previous frames: a hit that straddles a frame
        // boundary still shows its full rise.
        let prev = if t >= 2 { log[t - 1].min(log[t - 2]) } else { log[t - 1] };
        out[t] = (log[t] - prev).max(0.0);
    }
    let scale = percentile(&out, 0.98).max(1e-6);
    out.iter_mut().for_each(|v| *v = (*v / scale).min(1.5));
    out
}

/// Local maxima of a band's flux: at least `min_v`, the largest within +-20 ms, and 40 ms
/// apart.
fn peaks(f: &[f32], min_v: f32) -> Vec<usize> {
    let mut out = Vec::new();
    let mut last: isize = -100;
    for t in 2..f.len().saturating_sub(2) {
        let v = f[t];
        if v < min_v || (t as isize - last) < 4 || (t - 2..=t + 2).any(|u| f[u] > v) {
            continue;
        }
        out.push(t);
        last = t as isize;
    }
    out
}

/// Transcribe a mono drum stem at 44.1 kHz.
pub fn transcribe(x: &[f32]) -> DrumEvents {
    if x.len() < N_FFT {
        return DrumEvents { version: VERSION, onsets: Vec::new() };
    }
    let bands = band_powers(x);
    let (fk, fs, fw, fh) = (flux(&bands.kick), flux(&bands.snare), flux(&bands.wires), flux(&bands.hat));
    let n = fk.len();

    // Candidate hit times: the peaks of every band, merged when closer than 30 ms.
    let mut times: Vec<usize> = Vec::new();
    for band in [&fk, &fs, &fw, &fh] {
        times.extend(peaks(band, 0.2));
    }
    times.sort_unstable();
    let mut merged: Vec<usize> = Vec::new();
    for t in times {
        match merged.last_mut() {
            Some(last) if t - *last <= 3 => {}
            _ => merged.push(t),
        }
    }

    let mut onsets = Vec::new();
    for t in merged {
        let near = |f: &[f32]| f[t.saturating_sub(2)..(t + 3).min(n)].iter().cloned().fold(0.0, f32::max);
        let (k, s, w, h) = (near(&fk), near(&fs), near(&fw), near(&fh));
        // A kick is a rise in the lowest band that the snare body does not out-rise; a snare
        // needs body and wires; a hi-hat is high-band energy without a snare under it (a
        // snare's own wires reach that high, a hat played with it must be loud to count).
        // (a snare or hat with some low-band bleed has more wires than thump: not a kick)
        let kick = if k >= 0.45 && k >= 0.8 * s && k >= 0.6 * w { k } else { 0.0 };
        let snare = if s >= 0.3 && w >= 0.25 && s >= 0.9 * k { 0.5 * (s + w) } else { 0.0 };
        let hat = if h >= 0.2 && (snare == 0.0 || h >= 0.9) { h } else { 0.0 };
        if kick == 0.0 && snare == 0.0 && hat == 0.0 {
            continue;
        }
        let q = |v: f32| (v * 100.0).round().clamp(0.0, 100.0) as u8;
        onsets.push(DrumOnset { t_ms: (t * HOP * 1000 / RATE as usize) as u32, kick: q(kick), snare: q(snare), hat: q(hat) });
    }
    // One drum cannot be struck twice within 80 ms (a hi-hat within 60): the bands of one hit
    // sometimes peak a few frames apart, and the weaker echo would show as a double stroke.
    let mut last = [-1_000_000i64; 3];
    for o in onsets.iter_mut() {
        let t = o.t_ms as i64;
        for (i, (v, gap)) in [(&mut o.kick, 80), (&mut o.snare, 80), (&mut o.hat, 60)].into_iter().enumerate() {
            if *v == 0 {
                continue;
            }
            if t - last[i] < gap {
                *v = 0;
            } else {
                last[i] = t;
            }
        }
    }
    onsets.retain(|o| o.kick > 0 || o.snare > 0 || o.hat > 0);
    DrumEvents { version: VERSION, onsets }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hit(buf: &mut [f32], at: usize, kind: &str) {
        let mut rng = 12345u32;
        let mut noise = || {
            rng = rng.wrapping_mul(1664525).wrapping_add(1013904223);
            (rng >> 8) as f32 / (1u32 << 24) as f32 * 2.0 - 1.0
        };
        let n = (RATE as f32 * 0.25) as usize;
        let mut prev = 0.0f32;
        let mut prev2 = 0.0f32;
        for i in 0..n {
            let t = i as f32 / RATE as f32;
            let v = match kind {
                "kick" => {
                    let f = 50.0 + 70.0 * (-t * 25.0).exp();
                    (2.0 * std::f32::consts::PI * f * t).sin() * (-t * 9.0).exp() * 0.9
                }
                "snare" => {
                    let body = (2.0 * std::f32::consts::PI * 200.0 * t).sin() * (-t * 20.0).exp() * 0.5;
                    body + noise() * (-t * 14.0).exp() * 0.5
                }
                _ => {
                    // hi-hat: white noise through two differences (a steep high-pass), short.
                    let w = noise();
                    let hp = w - 2.0 * prev + prev2;
                    prev2 = prev;
                    prev = w;
                    hp * (-t * 45.0).exp() * 0.25
                }
            };
            if at + i < buf.len() {
                buf[at + i] += v;
            }
        }
    }

    #[test]
    fn finds_a_pop_pattern() {
        // 100 BPM, 4/4, eight bars: kick on 1 and 3, snare on 2 and 4, hi-hat eighths.
        let beat = (RATE as f32 * 0.6) as usize;
        let bars = 8;
        let mut x = vec![0f32; beat * 4 * bars + RATE as usize];
        let mut expect = Vec::new();
        for bar in 0..bars {
            for b in 0..4 {
                let at = (bar * 4 + b) * beat;
                if b % 2 == 0 {
                    hit(&mut x, at, "kick");
                } else {
                    hit(&mut x, at, "snare");
                }
                hit(&mut x, at, "hat");
                hit(&mut x, at + beat / 2, "hat");
                expect.push((at, b % 2 == 0, b % 2 == 1, true));
                expect.push((at + beat / 2, false, false, true));
            }
        }
        let ev = transcribe(&x);
        let tol = 30; // ms
        let mut found = 0;
        let mut wrong = 0;
        for (at, kick, snare, hat) in &expect {
            let t = (*at * 1000 / RATE as usize) as u32;
            match ev.onsets.iter().find(|o| (o.t_ms as i64 - t as i64).unsigned_abs() <= tol) {
                Some(o) => {
                    found += 1;
                    if (o.kick >= 25) != *kick || (o.snare >= 25) != *snare || (o.hat >= 25) != *hat {
                        wrong += 1;
                    }
                }
                None => {}
            }
        }
        let extra = ev.onsets.len().saturating_sub(found);
        assert!(found >= expect.len() * 95 / 100, "recall {found}/{}", expect.len());
        assert!(wrong <= expect.len() / 20, "misclassified {wrong} of {found}: {:?}", &ev.onsets[..8.min(ev.onsets.len())]);
        assert!(extra <= expect.len() / 10, "{extra} spurious hits");
    }

    #[test]
    fn silence_has_no_hits() {
        assert!(transcribe(&vec![0.0; RATE as usize * 3]).onsets.is_empty());
    }
}
