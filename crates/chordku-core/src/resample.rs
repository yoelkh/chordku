//! Sample-rate conversion on top of rubato's band-limited sinc resampler.
//!
//! The ratio is free (not locked to integer rates) so the analysis path can fold
//! a tuning correction into the same pass: a song tuned `c` cents sharp is
//! resampled by an extra factor `2^(c/1200)`, which lowers its pitch back onto
//! the A440 grid the chord model was trained on, at zero additional cost.

use anyhow::{Context, Result};
use rubato::audioadapter_buffers::direct::InterleavedSlice;
use rubato::{Async, FixedAsync, Resampler, SincInterpolationParameters, WindowFunction};

/// Resample interleaved audio by `ratio = out_rate / in_rate`.
pub fn resample_interleaved(input: &[f32], channels: usize, ratio: f64) -> Result<Vec<f32>> {
    if channels == 0 || input.is_empty() {
        return Ok(Vec::new());
    }
    if (ratio - 1.0).abs() < 1e-9 {
        return Ok(input.to_vec());
    }
    let frames = input.len() / channels;
    let params = SincInterpolationParameters::new(128, WindowFunction::BlackmanHarris2);
    let mut rs = Async::<f32>::new_sinc(ratio, 1.1, &params, 4096, channels, FixedAsync::Input)
        .context("create resampler")?;
    let adapter = InterleavedSlice::new(input, channels, frames).context("wrap input")?;
    let out = rs.process_all(&adapter, frames, None).context("resample")?;
    Ok(out.take_data())
}

/// Mono convenience wrapper.
pub fn resample_mono(input: &[f32], in_rate: u32, out_rate: u32, tuning_cents: f32) -> Result<Vec<f32>> {
    resample_interleaved(input, 1, ratio_with_tuning(in_rate, out_rate, tuning_cents))
}

/// `out/in`, stretched so that audio tuned `tuning_cents` away from A440 lands on A440.
pub fn ratio_with_tuning(in_rate: u32, out_rate: u32, tuning_cents: f32) -> f64 {
    (out_rate as f64 / in_rate as f64) * 2f64.powf(tuning_cents as f64 / 1200.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn halves_length_and_keeps_tone() {
        let sr = 44_100u32;
        let x: Vec<f32> =
            (0..sr).map(|n| (2.0 * std::f32::consts::PI * 440.0 * n as f32 / sr as f32).sin()).collect();
        let y = resample_mono(&x, sr, 22_050, 0.0).unwrap();
        assert!((y.len() as i64 - 22_050).abs() <= 2, "len {}", y.len());
        // Zero crossings of a 440 Hz tone over ~1 s: about 880.
        let zc = y.windows(2).filter(|w| (w[0] < 0.0) != (w[1] < 0.0)).count();
        assert!((zc as i64 - 880).abs() <= 4, "zero crossings {zc}");
        let peak = y[2000..20_000].iter().fold(0f32, |m, v| m.max(v.abs()));
        assert!((peak - 1.0).abs() < 0.02, "peak {peak}");
    }
}
