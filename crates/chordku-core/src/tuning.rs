//! Global tuning estimation: how many cents the recording deviates from A4 = 440 Hz.
//!
//! The chord model's CQT bins are centred on equal-tempered pitches at A440. Old tape
//! transfers, detuned guitars and sped-up uploads routinely sit 20-40 cents off, which
//! smears every note across two bins. We measure the offset from spectral peaks and
//! fold the correction into the analysis resampler (see `resample::ratio_with_tuning`).
//!
//! Method: Hann STFT (8192 @ 22.05 kHz), parabolic peak interpolation, deviation of each
//! peak from the nearest semitone, magnitude-weighted circular mean over the song.

use std::f32::consts::PI;

use realfft::RealFftPlanner;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tuning {
    /// Offset in cents, in `[-50, 50)`. Positive means the recording is sharp.
    pub cents: f32,
    /// Resultant length of the circular mean, `0..=1`. Below ~0.15 the estimate is noise.
    pub confidence: f32,
}

impl Tuning {
    /// The correction worth applying: zero when unreliable or small.
    ///
    /// Measured on GuitarSet (docs/ACCURACY.md): in-tune guitars read +8 cents (plucked
    /// strings sound sharp, and the chord model was trained on exactly such audio), and
    /// correcting offsets of 8-12 cents changes nothing (-0.05 points). On recordings
    /// detuned by 30-40 cents the correction recovers 6-8 points. Hence the 15 cent floor.
    pub fn correction_cents(&self) -> f32 {
        if self.confidence >= 0.15 && self.cents.abs() >= 15.0 {
            self.cents
        } else {
            0.0
        }
    }
}

const N_FFT: usize = 8192;
const HOP: usize = 4096;
const F_LO: f32 = 80.0;
const F_HI: f32 = 2000.0;

pub fn estimate(y: &[f32], sample_rate: u32) -> Tuning {
    estimate_in_band(y, sample_rate, F_LO, F_HI)
}

/// [`estimate`] restricted to spectral peaks between `f_lo` and `f_hi` Hz.
pub fn estimate_in_band(y: &[f32], sample_rate: u32, f_lo: f32, f_hi: f32) -> Tuning {
    if y.len() < N_FFT {
        return Tuning { cents: 0.0, confidence: 0.0 };
    }
    let mut planner = RealFftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(N_FFT);
    let window: Vec<f32> = (0..N_FFT).map(|i| 0.5 - 0.5 * (2.0 * PI * i as f32 / N_FFT as f32).cos()).collect();
    let mut frame = fft.make_input_vec();
    let mut spec = fft.make_output_vec();
    let mut scratch = fft.make_scratch_vec();
    let mut mag = vec![0f32; N_FFT / 2 + 1];

    let bin_hz = sample_rate as f32 / N_FFT as f32;
    let lo = (f_lo / bin_hz).ceil() as usize;
    let hi = ((f_hi / bin_hz).floor() as usize).min(N_FFT / 2 - 1);

    let (mut sx, mut sy, mut sw) = (0f64, 0f64, 0f64);
    let mut start = 0;
    while start + N_FFT <= y.len() {
        for i in 0..N_FFT {
            frame[i] = y[start + i] * window[i];
        }
        fft.process_with_scratch(&mut frame, &mut spec, &mut scratch).expect("planned sizes");
        let mut peak_mag = 0f32;
        for (m, c) in mag.iter_mut().zip(&spec) {
            *m = c.norm();
            peak_mag = peak_mag.max(*m);
        }
        let floor = peak_mag * 0.05;
        if peak_mag > 1e-4 {
            for k in lo.max(1)..hi {
                let (a, b, c) = (mag[k - 1], mag[k], mag[k + 1]);
                if b <= floor || b < a || b <= c {
                    continue;
                }
                // Parabolic interpolation on log magnitude.
                let (la, lb, lc) = (a.max(1e-12).ln(), b.ln(), c.max(1e-12).ln());
                let denom = la - 2.0 * lb + lc;
                let delta = if denom.abs() > 1e-12 { 0.5 * (la - lc) / denom } else { 0.0 };
                let freq = (k as f32 + delta.clamp(-0.5, 0.5)) * bin_hz;
                let semis = 12.0 * (freq / 440.0).log2();
                let dev = semis - semis.round(); // [-0.5, 0.5) semitones
                let theta = (2.0 * PI * dev) as f64;
                let w = b.sqrt() as f64; // compress so one loud partial cannot dominate
                sx += w * theta.cos();
                sy += w * theta.sin();
                sw += w;
            }
        }
        start += HOP;
    }
    if sw <= 0.0 {
        return Tuning { cents: 0.0, confidence: 0.0 };
    }
    let cents = (sy.atan2(sx) / (2.0 * std::f64::consts::PI) * 100.0) as f32;
    let confidence = ((sx * sx + sy * sy).sqrt() / sw) as f32;
    Tuning { cents, confidence }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chord(cents: f32, secs: f32) -> Vec<f32> {
        let sr = 22_050f32;
        let shift = 2f32.powf(cents / 1200.0);
        let notes = [220.0f32, 261.63, 329.63, 440.0];
        (0..(sr * secs) as usize)
            .map(|n| {
                let t = n as f32 / sr;
                notes
                    .iter()
                    .map(|f| {
                        (1..4).map(|h| (2.0 * PI * f * shift * h as f32 * t).sin() / h as f32).sum::<f32>()
                    })
                    .sum::<f32>()
                    * 0.1
            })
            .collect()
    }

    #[test]
    fn measures_known_offsets() {
        for want in [-35.0f32, -12.0, 0.0, 20.0, 41.0] {
            let got = estimate(&chord(want, 6.0), 22_050);
            assert!((got.cents - want).abs() < 3.0, "want {want} got {:?}", got);
            assert!(got.confidence > 0.6, "{got:?}");
        }
    }

    #[test]
    fn small_or_unreliable_offsets_are_not_corrected() {
        assert_eq!(Tuning { cents: 12.0, confidence: 0.9 }.correction_cents(), 0.0);
        assert_eq!(Tuning { cents: 30.0, confidence: 0.05 }.correction_cents(), 0.0);
        assert_eq!(Tuning { cents: -30.0, confidence: 0.5 }.correction_cents(), -30.0);
    }
}
