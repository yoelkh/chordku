//! Hybrid Transformer Demucs (Rouard, Massa, Defossez, ICASSP 2023) through ONNX Runtime.
//!
//! The graph is Intel's export (huggingface.co/Intel/demucs-openvino, `htdemucs_fwd.onnx`)
//! which contains only the network: STFT, normalisation, masking, iSTFT, segmenting and
//! overlap-add are done here, mirroring `demucs.htdemucs.HTDemucs.forward`,
//! `demucs.apply.apply_model` (overlap 0.25, transition power 1, no shifts) and the
//! track-level normalisation in `demucs.separate`.
//!
//! The pretrained weights are provided by their authors for research purposes only, so
//! they are downloaded by the user on first use and never redistributed with ChordKu.
//!
//! Output is streamed: a block of finished samples is handed to the sink after every
//! 7.8 s segment, so memory stays flat and the caller can encode, report progress,
//! yield to more urgent work or cancel between segments.

use std::ops::ControlFlow;
use std::path::Path;

use anyhow::{ensure, Result};
use ort::session::Session;
use ort::value::Tensor;

use super::stft::{Stft, BINS};
use super::{Separator, StemBlock, N_STEMS, STEM_RATE};
use crate::ortutil::{session, Device};

/// 7.8 s at 44.1 kHz: the training segment of the pretrained model.
pub const SEGMENT: usize = 343_980;
const OVERLAP: f64 = 0.25;
const STRIDE: usize = ((1.0 - OVERLAP) * SEGMENT as f64) as usize; // 257_985
const FRAMES: usize = 336;
const CHANNELS: usize = 2;

pub struct HtDemucs {
    session: Session,
    stft: Stft,
    /// One transform per stem, so the eight inverse STFTs of a segment run four at a time.
    /// On a GPU the network takes half a second per segment; serial DSP would double that.
    stem_stfts: Vec<Stft>,
}

impl HtDemucs {
    pub fn load(onnx: &Path, threads: usize) -> Result<Self> {
        Self::load_on(onnx, Device::Cpu, threads)
    }

    pub fn load_on(onnx: &Path, device: Device, threads: usize) -> Result<Self> {
        Ok(Self {
            session: session(onnx, device, threads, true)?,
            stft: Stft::new(),
            stem_stfts: (0..N_STEMS).map(|_| Stft::new()).collect(),
        })
    }

    /// Separate one full-length segment. `mix` is planar stereo, `SEGMENT` samples each.
    /// Returns `[stem][channel] -> SEGMENT samples`.
    fn run_segment(&mut self, mix: &[Vec<f32>; CHANNELS]) -> Result<Vec<[Vec<f32>; CHANNELS]>> {
        let plane = BINS * FRAMES;

        // Spectrogram branch: complex-as-channels [ch0.re, ch0.im, ch1.re, ch1.im].
        let mut x = vec![0f32; 2 * CHANNELS * plane];
        for (c, channel) in mix.iter().enumerate() {
            let (re, rest) = x[2 * c * plane..].split_at_mut(plane);
            self.stft.forward(channel, re, &mut rest[..plane]);
        }
        let (mean, std) = mean_std(x.iter().copied());
        let inv = 1.0 / (1e-5 + std);
        x.iter_mut().for_each(|v| *v = (*v - mean) * inv);

        // Waveform branch.
        let mut xt = Vec::with_capacity(CHANNELS * SEGMENT);
        xt.extend_from_slice(&mix[0]);
        xt.extend_from_slice(&mix[1]);
        let (mean_t, std_t) = mean_std(xt.iter().copied());
        let inv_t = 1.0 / (1e-5 + std_t);
        xt.iter_mut().for_each(|v| *v = (*v - mean_t) * inv_t);

        let outputs = self.session.run(ort::inputs![
            "x" => Tensor::from_array(([1usize, 2 * CHANNELS, BINS, FRAMES], x))?,
            "xt" => Tensor::from_array(([1usize, CHANNELS, SEGMENT], xt))?,
        ])?;
        let (_, x_out) = outputs["x_out"].try_extract_tensor::<f32>()?;
        let (_, xt_out) = outputs["xt_out"].try_extract_tensor::<f32>()?;
        ensure!(x_out.len() == N_STEMS * 2 * CHANNELS * plane, "unexpected x_out size {}", x_out.len());
        ensure!(xt_out.len() == N_STEMS * CHANNELS * SEGMENT, "unexpected xt_out size {}", xt_out.len());

        // One thread per stem: denormalise, inverse STFT, add the waveform branch.
        let stems: Vec<[Vec<f32>; CHANNELS]> = std::thread::scope(|scope| {
            let handles: Vec<_> = self
                .stem_stfts
                .iter_mut()
                .enumerate()
                .map(|(s, stft)| {
                    scope.spawn(move || {
                        let mut re = vec![0f32; plane];
                        let mut im = vec![0f32; plane];
                        let mut pair = [vec![0f32; SEGMENT], vec![0f32; SEGMENT]];
                        for (c, out) in pair.iter_mut().enumerate() {
                            let base = (s * 2 * CHANNELS + 2 * c) * plane;
                            for (d, v) in re.iter_mut().zip(&x_out[base..base + plane]) {
                                *d = v * std + mean;
                            }
                            for (d, v) in im.iter_mut().zip(&x_out[base + plane..base + 2 * plane]) {
                                *d = v * std + mean;
                            }
                            stft.inverse(&re, &im, out);
                            let t = &xt_out[(s * CHANNELS + c) * SEGMENT..(s * CHANNELS + c + 1) * SEGMENT];
                            for (o, v) in out.iter_mut().zip(t) {
                                *o += v * std_t + mean_t;
                            }
                        }
                        pair
                    })
                })
                .collect();
            handles.into_iter().map(|h| h.join().expect("stem thread")).collect()
        });
        Ok(stems)
    }
}

impl Separator for HtDemucs {
    fn separate(
        &mut self,
        mix: &[Vec<f32>; CHANNELS],
        sink: &mut dyn FnMut(StemBlock<'_>) -> Result<ControlFlow<()>>,
    ) -> Result<bool> {
        let total = mix[0].len();
        ensure!(mix[1].len() == total, "channels differ in length");
        if total == 0 {
            return Ok(true);
        }
        let _ = STEM_RATE;

        // Track-level normalisation by the mono reference (demucs.separate).
        let (ref_mean, ref_std) = mean_std((0..total).map(|i| 0.5 * (mix[0][i] + mix[1][i])));
        let ref_std = if ref_std > 1e-8 { ref_std } else { 1.0 };

        // Transition weights: a triangle over the segment, normalised to peak 1.
        let half = SEGMENT / 2;
        let peak = (SEGMENT - half) as f32;
        let weight = |n: usize| -> f32 {
            if n < half {
                (n + 1) as f32 / peak
            } else {
                (SEGMENT - n) as f32 / peak
            }
        };

        let offsets: Vec<usize> = (0..total).step_by(STRIDE).collect();
        let carry_len = SEGMENT - STRIDE;
        let mut carry = vec![[vec![0f32; carry_len], vec![0f32; carry_len]]; N_STEMS];
        let mut carry_w = vec![0f32; carry_len];
        let mut padded = [vec![0f32; SEGMENT], vec![0f32; SEGMENT]];

        for (i, &offset) in offsets.iter().enumerate() {
            let length = (total - offset).min(SEGMENT);
            // TensorChunk.padded: centre the chunk in a full segment, filling with real
            // neighbouring audio where it exists and zeros beyond the ends of the track.
            let delta = SEGMENT - length;
            let start = offset as i64 - (delta / 2) as i64;
            for (c, dst) in padded.iter_mut().enumerate() {
                for (k, slot) in dst.iter_mut().enumerate() {
                    let j = start + k as i64;
                    *slot = if j >= 0 && (j as usize) < total { (mix[c][j as usize] - ref_mean) / ref_std } else { 0.0 };
                }
            }
            let stems = self.run_segment(&padded)?;
            let trim = delta / 2; // center_trim

            let last = i + 1 == offsets.len();
            let emit = if last { length } else { STRIDE.min(length) };
            let mut block: Vec<[Vec<f32>; CHANNELS]> =
                (0..N_STEMS).map(|_| [Vec::with_capacity(emit), Vec::with_capacity(emit)]).collect();
            let mut next_carry = vec![[vec![0f32; carry_len], vec![0f32; carry_len]]; N_STEMS];
            let mut next_carry_w = vec![0f32; carry_len];
            for n in 0..length {
                let w = weight(n);
                let (cw, has_carry) = if n < carry_len { (carry_w[n], true) } else { (0.0, false) };
                let wsum = w + cw;
                for s in 0..N_STEMS {
                    for c in 0..CHANNELS {
                        let acc = w * stems[s][c][trim + n] + if has_carry { carry[s][c][n] } else { 0.0 };
                        if n < emit {
                            block[s][c].push(acc / wsum * ref_std + ref_mean);
                        } else {
                            next_carry[s][c][n - emit] = acc;
                        }
                    }
                }
                if n >= emit {
                    next_carry_w[n - emit] = wsum;
                }
            }
            carry = next_carry;
            carry_w = next_carry_w;

            let flow = sink(StemBlock { offset, stems: &block, segment: i + 1, segments: offsets.len() })?;
            if flow.is_break() {
                return Ok(false);
            }
        }
        Ok(true)
    }
}

/// Mean and unbiased standard deviation (what `torch.Tensor.std` computes), in f64.
fn mean_std(values: impl Iterator<Item = f32> + Clone) -> (f32, f32) {
    let (mut n, mut sum) = (0u64, 0f64);
    for v in values.clone() {
        sum += v as f64;
        n += 1;
    }
    if n < 2 {
        return (sum as f32, 0.0);
    }
    let mean = sum / n as f64;
    let var = values.map(|v| (v as f64 - mean).powi(2)).sum::<f64>() / (n - 1) as f64;
    (mean as f32, var.sqrt() as f32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn geometry_matches_demucs() {
        assert_eq!(STRIDE, 257_985);
        assert_eq!(Stft::frames_for(SEGMENT), FRAMES);
    }

    #[test]
    fn mean_std_is_unbiased() {
        let (m, s) = mean_std([1.0f32, 2.0, 3.0, 4.0].into_iter());
        assert!((m - 2.5).abs() < 1e-6);
        assert!((s - 1.290_994_4).abs() < 1e-6);
    }
}
