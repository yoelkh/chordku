//! The exact STFT / iSTFT pair Hybrid Transformer Demucs wraps around its network
//! (`demucs.htdemucs.HTDemucs._spec` / `_ispec`, `demucs.spec.spectro` / `ispectro`):
//!
//!   n_fft 4096, hop 1024, periodic Hann, `normalized=True`, `center=True` (reflect)
//!   signal reflect-padded by 1536 left and `1536 + le*hop - len` right (le = ceil(len/hop))
//!   frames [2, 2 + le) and bins [0, 2048) are kept: the Nyquist bin is dropped
//!
//! The inverse re-inserts the zero bin and the two zero frames per side, overlap-adds and
//! divides by the window envelope, which is the constant 1.5 everywhere we read from.

use std::f32::consts::PI;
use std::sync::Arc;

use realfft::num_complex::Complex;
use realfft::{ComplexToReal, RealFftPlanner, RealToComplex};

pub const N_FFT: usize = 4096;
pub const HOP: usize = 1024;
pub const BINS: usize = N_FFT / 2; // Nyquist dropped
const PAD: usize = HOP / 2 * 3; // 1536
const EDGE_FRAMES: usize = 2;

pub struct Stft {
    window: Vec<f32>,
    forward: Arc<dyn RealToComplex<f32>>,
    inverse: Arc<dyn ComplexToReal<f32>>,
    // scratch
    frame: Vec<f32>,
    spec: Vec<Complex<f32>>,
    fwd_scratch: Vec<Complex<f32>>,
    inv_scratch: Vec<Complex<f32>>,
    ola: Vec<f32>,
}

impl Default for Stft {
    fn default() -> Self {
        Self::new()
    }
}

impl Stft {
    pub fn new() -> Self {
        let mut planner = RealFftPlanner::<f32>::new();
        let forward = planner.plan_fft_forward(N_FFT);
        let inverse = planner.plan_fft_inverse(N_FFT);
        Self {
            window: (0..N_FFT).map(|i| 0.5 - 0.5 * (2.0 * PI * i as f32 / N_FFT as f32).cos()).collect(),
            frame: forward.make_input_vec(),
            spec: forward.make_output_vec(),
            fwd_scratch: forward.make_scratch_vec(),
            inv_scratch: inverse.make_scratch_vec(),
            ola: Vec::new(),
            forward,
            inverse,
        }
    }

    /// Frames produced for a signal of `len` samples.
    pub fn frames_for(len: usize) -> usize {
        len.div_ceil(HOP)
    }

    /// Forward transform of one channel. `re` and `im` are `[BINS, frames]` row-major
    /// (frequency major), the layout of the network's spectrogram input.
    pub fn forward(&mut self, x: &[f32], re: &mut [f32], im: &mut [f32]) {
        let frames = Self::frames_for(x.len());
        assert!(re.len() == BINS * frames && im.len() == BINS * frames);
        let n = x.len() as i64;
        let padded_len = (PAD + frames * HOP + PAD) as i64; // after the outer reflect pad
        let norm = 1.0 / (N_FFT as f32).sqrt();

        // sample `j` of the outer-padded signal
        let outer = |j: i64| -> f32 {
            let i = j - PAD as i64; // index into x, may be out of range on both sides
            x[reflect(i, n)]
        };
        for t in 0..frames {
            // Frame `t + 2` of the centred STFT over the outer-padded signal starts at
            // (t + 2) * HOP - N_FFT / 2 in outer-padded coordinates.
            let start = ((t + EDGE_FRAMES) * HOP) as i64 - (N_FFT / 2) as i64;
            for k in 0..N_FFT {
                let j = reflect(start + k as i64, padded_len) as i64;
                self.frame[k] = outer(j) * self.window[k];
            }
            self.forward
                .process_with_scratch(&mut self.frame, &mut self.spec, &mut self.fwd_scratch)
                .expect("planned sizes");
            for f in 0..BINS {
                re[f * frames + t] = self.spec[f].re * norm;
                im[f * frames + t] = self.spec[f].im * norm;
            }
        }
    }

    /// Inverse transform of one channel into `out` (`out.len()` = original signal length).
    pub fn inverse(&mut self, re: &[f32], im: &[f32], out: &mut [f32]) {
        let len = out.len();
        let frames = Self::frames_for(len);
        assert!(re.len() == BINS * frames && im.len() == BINS * frames);
        let total_frames = frames + 2 * EDGE_FRAMES;
        let ola_len = (total_frames - 1) * HOP + N_FFT;
        self.ola.clear();
        self.ola.resize(ola_len, 0.0);
        let norm = 1.0 / (N_FFT as f32).sqrt();

        for t in 0..frames {
            for f in 0..BINS {
                self.spec[f] = Complex::new(re[f * frames + t], im[f * frames + t]);
            }
            self.spec[BINS] = Complex::new(0.0, 0.0);
            // A real signal has a real DC bin. The network does not guarantee that, torch's
            // irfft silently ignores the imaginary part, and realfft rejects it.
            self.spec[0].im = 0.0;
            self.inverse
                .process_with_scratch(&mut self.spec, &mut self.frame, &mut self.inv_scratch)
                .expect("planned sizes");
            let at = (t + EDGE_FRAMES) * HOP;
            for k in 0..N_FFT {
                self.ola[at + k] += self.frame[k] * norm * self.window[k];
            }
        }
        // Window envelope: sum of squared periodic Hann at hop N/4 is exactly 1.5 wherever
        // four frames overlap, which holds for the whole region read below because the
        // zero edge frames count towards the envelope too.
        let first = N_FFT / 2 + PAD;
        let inv_env = 1.0 / 1.5;
        for (o, v) in out.iter_mut().zip(&self.ola[first..first + len]) {
            *o = v * inv_env;
        }
    }
}

/// numpy / torch "reflect" index mapping for any out-of-range distance.
fn reflect(i: i64, n: i64) -> usize {
    if n <= 1 {
        return 0;
    }
    let period = 2 * (n - 1);
    let mut k = i.rem_euclid(period);
    if k >= n {
        k = period - k;
    }
    k as usize
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_is_transparent() {
        // The Nyquist bin is discarded, so use a band-limited signal as the network does.
        let len = 343_980 / 8;
        let x: Vec<f32> = (0..len)
            .map(|n| {
                let t = n as f32 / 44_100.0;
                0.4 * (2.0 * PI * 220.0 * t).sin() + 0.2 * (2.0 * PI * 3520.0 * t + 1.0).sin() + 0.1 * (2.0 * PI * 9000.0 * t).cos()
            })
            .collect();
        let frames = Stft::frames_for(len);
        let mut stft = Stft::new();
        let (mut re, mut im) = (vec![0.0; BINS * frames], vec![0.0; BINS * frames]);
        stft.forward(&x, &mut re, &mut im);
        let mut y = vec![0.0; len];
        stft.inverse(&re, &im, &mut y);
        // Demucs discards the two outermost frames on each side, so the first and last 1536
        // samples are only partly rebuilt by design (the network is trained that way and the
        // overlap-add in `htdemucs` gives segment edges almost no weight). Everything in
        // between must be exact.
        let inner = PAD..frames * HOP - PAD;
        let err = x[inner.clone()].iter().zip(&y[inner]).map(|(a, b)| (a - b).abs()).fold(0f32, f32::max);
        assert!(err < 5e-5, "max round-trip error {err}"); // f32 FFT precision
        assert!((y[0] - x[0]).abs() > 1e-3, "edges are expected to be attenuated");
    }

    #[test]
    fn segment_has_336_frames() {
        assert_eq!(Stft::frames_for(343_980), 336);
    }
}
