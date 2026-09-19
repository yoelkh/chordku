//! Constant-Q transform, numerically matched to `librosa.cqt(..., scale=True)`.
//!
//! librosa evaluates the CQT with recursive down-sampling; we evaluate the same
//! wavelet bank directly at the full rate. One real FFT per frame plus a sparse
//! frequency-domain product per bin is ~50x cheaper than a time-domain filter bank
//! and needs no resampler, so there is no octave-dependent error to tune away.
//! Against librosa 0.11 on real music the log-magnitude differs by 0.0001 on
//! average (corr 0.9991) and BTC chord labels agree on 99.3% of frames
//! (tools/export/proto_decomp.py).
//!
//! Filter definition (librosa.filters.wavelet, filter_scale=1, gamma=0, hann, norm=1):
//!   alpha = (r^2 - 1) / (r^2 + 1), r = 2^(1/bins_per_octave);  Q = 1 / alpha
//!   L_k   = Q * sr / f_k
//!   h_k[n] = w[n] * exp(2*pi*i * f_k * n / sr),  n in [floor(-L_k/2), floor(L_k/2)),  sum|h_k| = 1
//! and the response is `C[k, t] = sqrt(L_k) * sum_n x[t*hop + n] * h_k[n]`.

use std::f64::consts::PI;
use std::sync::Arc;

use realfft::num_complex::Complex;
use realfft::{RealFftPlanner, RealToComplex};

/// How the signal is extended past its ends before framing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PadMode {
    /// Zeros (librosa >= 0.10 default).
    Constant,
    /// Mirror without repeating the edge sample (librosa <= 0.9 default, which BTC was trained with).
    Reflect,
}

#[derive(Clone, Copy, Debug)]
pub struct CqtParams {
    pub sample_rate: u32,
    pub hop: usize,
    pub n_bins: usize,
    pub bins_per_octave: usize,
    pub fmin: f64,
}

impl CqtParams {
    /// The BTC front-end: 22.05 kHz, hop 2048, 144 bins, 24 per octave, from C1.
    pub const BTC: CqtParams = CqtParams {
        sample_rate: 22_050,
        hop: 2048,
        n_bins: 144,
        bins_per_octave: 24,
        fmin: 32.703_195_662_574_83, // C1
    };
}

struct SparseRow {
    /// First FFT bin this row touches.
    start: usize,
    /// Frequency response on `[start, start + coeffs.len())`, already scaled by sqrt(L)/n_fft.
    coeffs: Vec<Complex<f32>>,
}

/// A reusable CQT plan (filter bank + FFT).
pub struct Cqt {
    params: CqtParams,
    n_fft: usize,
    rows: Vec<SparseRow>,
    fft: Arc<dyn RealToComplex<f32>>,
}

impl Cqt {
    pub fn new(params: CqtParams) -> Self {
        let sr = params.sample_rate as f64;
        let r = 2f64.powf(1.0 / params.bins_per_octave as f64);
        let alpha = (r * r - 1.0) / (r * r + 1.0);
        let q = 1.0 / alpha;
        let freqs: Vec<f64> = (0..params.n_bins)
            .map(|k| params.fmin * 2f64.powf(k as f64 / params.bins_per_octave as f64))
            .collect();
        let lengths: Vec<f64> = freqs.iter().map(|f| q * sr / f).collect();
        let max_len = lengths.iter().cloned().fold(0.0, f64::max);
        let n_fft = (max_len.ceil() as usize).next_power_of_two();
        let half = n_fft / 2;

        let mut planner = RealFftPlanner::<f32>::new();
        let fft = planner.plan_fft_forward(n_fft);

        // Build each wavelet in f64 exactly as librosa does (centred in an n_fft frame), take
        // its spectrum, and keep only the band where a Hann-windowed tone has energy.
        let mut cplanner = rustfft::FftPlanner::<f64>::new();
        let cfft = cplanner.plan_fft_forward(n_fft);
        let mut buf = vec![Complex::new(0f64, 0f64); n_fft];
        let mut rows = Vec::with_capacity(params.n_bins);
        for (&f, &len) in freqs.iter().zip(&lengths) {
            let n0 = (-len / 2.0).floor() as i64;
            let n1 = (len / 2.0).floor() as i64;
            let m = (n1 - n0) as usize;
            // Periodic Hann (scipy get_window(..., fftbins=True)); |exp(i..)| = 1 so the
            // L1 norm of the wavelet is simply the window sum.
            let win: Vec<f64> = (0..m).map(|i| 0.5 - 0.5 * (2.0 * PI * i as f64 / m as f64).cos()).collect();
            let l1: f64 = win.iter().sum();
            let offset = (n_fft - m) / 2; // librosa.util.pad_center
            buf.fill(Complex::new(0.0, 0.0));
            let wf = 2.0 * PI * f / sr;
            for (j, &w) in win.iter().enumerate() {
                let phase = wf * (n0 + j as i64) as f64;
                buf[offset + j] = Complex::new(w * phase.cos(), w * phase.sin());
            }
            cfft.process(&mut buf);

            let centre_bin = f * n_fft as f64 / sr;
            // Hann main lobe half-width is 2 * n_fft / m bins; keep four times that.
            let bw = (8.0 * n_fft as f64 / m as f64).ceil() as i64 + 2;
            let lo = ((centre_bin.floor() as i64) - bw).max(0) as usize;
            let hi = (((centre_bin.ceil() as i64) + bw) as usize).min(half);
            let scale = len.sqrt() / n_fft as f64 / l1;
            let coeffs = buf[lo..=hi]
                .iter()
                .map(|c| Complex::new((c.re * scale) as f32, (c.im * scale) as f32))
                .collect();
            rows.push(SparseRow { start: lo, coeffs });
        }
        Self { params, n_fft, rows, fft }
    }

    pub fn params(&self) -> &CqtParams {
        &self.params
    }

    pub fn n_fft(&self) -> usize {
        self.n_fft
    }

    /// Number of frames `librosa.cqt` yields for `n` samples (`center=True`).
    pub fn frames_for(&self, n: usize) -> usize {
        1 + n / self.params.hop
    }

    /// `ln(|CQT| + 1e-6)`, row-major `[frames, n_bins]`: exactly the BTC input feature.
    pub fn log_magnitude(&self, y: &[f32], pad: PadMode) -> Vec<f32> {
        let n_bins = self.params.n_bins;
        let hop = self.params.hop;
        let half = self.n_fft / 2;
        let frames = self.frames_for(y.len());
        let mut out = vec![0f32; frames * n_bins];
        if y.is_empty() {
            out.fill((1e-6f32).ln());
            return out;
        }

        let mut frame = self.fft.make_input_vec();
        let mut spec = self.fft.make_output_vec();
        let mut scratch = self.fft.make_scratch_vec();
        let n = y.len() as i64;
        for t in 0..frames {
            let start = (t * hop) as i64 - half as i64;
            for (i, slot) in frame.iter_mut().enumerate() {
                let idx = start + i as i64;
                *slot = if idx >= 0 && idx < n {
                    y[idx as usize]
                } else {
                    match pad {
                        PadMode::Constant => 0.0,
                        PadMode::Reflect => y[reflect_index(idx, n)],
                    }
                };
            }
            self.fft
                .process_with_scratch(&mut frame, &mut spec, &mut scratch)
                .expect("fft buffers are sized by the plan");
            let row_out = &mut out[t * n_bins..(t + 1) * n_bins];
            for (k, row) in self.rows.iter().enumerate() {
                let mut acc = Complex::new(0f32, 0f32);
                for (c, s) in row.coeffs.iter().zip(&spec[row.start..]) {
                    acc += c * s;
                }
                row_out[k] = (acc.norm() + 1e-6).ln();
            }
        }
        out
    }
}

/// numpy `mode="reflect"` index mapping, valid for any distance past the ends.
fn reflect_index(idx: i64, n: i64) -> usize {
    if n == 1 {
        return 0;
    }
    let period = 2 * (n - 1);
    let mut i = idx.rem_euclid(period);
    if i >= n {
        i = period - i;
    }
    i as usize
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reflect_matches_numpy() {
        // np.pad([0,1,2,3], 5, mode="reflect") -> [1,2,3,2,1, 0,1,2,3, 2,1,0,1,2]
        let got: Vec<usize> = (-5..9).map(|i| reflect_index(i, 4)).collect();
        assert_eq!(got, vec![1, 2, 3, 2, 1, 0, 1, 2, 3, 2, 1, 0, 1, 2]);
    }

    #[test]
    fn pure_tone_peaks_at_its_bin() {
        let cqt = Cqt::new(CqtParams::BTC);
        assert_eq!(cqt.n_fft(), 32_768);
        // A4 = 440 Hz is 45 semitones above C1 -> bin 90 at 24 bins per octave.
        let sr = 22_050f32;
        let y: Vec<f32> = (0..22_050 * 3).map(|n| (2.0 * std::f32::consts::PI * 440.0 * n as f32 / sr).sin()).collect();
        let feat = cqt.log_magnitude(&y, PadMode::Constant);
        let frames = cqt.frames_for(y.len());
        assert_eq!(feat.len(), frames * 144);
        let mid = &feat[(frames / 2) * 144..(frames / 2 + 1) * 144];
        let arg = mid.iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1)).unwrap().0;
        assert_eq!(arg, 90);
        // |C| = sqrt(L) * A/2 for a unit sine at the bin centre.
        let r = 2f64.powf(1.0 / 24.0);
        let q = (r * r + 1.0) / (r * r - 1.0);
        let expect = ((q * 22_050.0 / 440.0).sqrt() * 0.5).ln() as f32;
        assert!((mid[90] - expect).abs() < 0.02, "got {} expect {}", mid[90], expect);
    }
}
