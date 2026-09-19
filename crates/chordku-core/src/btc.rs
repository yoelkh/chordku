//! BTC chord inference (Park et al., ISMIR 2019, MIT) through ONNX Runtime.
//!
//! The model sees 10 s instances of 108 CQT frames. It was trained on instances cut
//! every 5 s with the CQT computed per instance (edges reflect-padded), so we feed it
//! exactly that: instances 54 hops (5.02 s) apart, each with its own CQT. Every output
//! frame is then covered by two instances; their log-probabilities are averaged with a
//! triangular weight so each frame is dominated by the instance in which it has the most
//! context. This removes the accuracy dip the reference script has at its hard 10 s seams.

use std::path::Path;

use anyhow::{ensure, Context, Result};
use ort::session::Session;
use ort::value::Tensor;
use serde::Deserialize;

use crate::chords::N_CLASSES;
use crate::cqt::{Cqt, CqtParams, PadMode};
use crate::ortutil::{session, Device};

pub const TIMESTEP: usize = 108;
pub const WINDOW_SAMPLES: usize = 220_500; // 10 s at 22.05 kHz
pub const WINDOW_HOP_FRAMES: usize = TIMESTEP / 2;

/// Sidecar written by tools/export/export_btc.py next to `btc.onnx`.
#[derive(Clone, Debug, Deserialize)]
pub struct BtcMeta {
    /// Feature mean of the checkpoint; padding frames are filled with it so they
    /// normalise to zero inside the graph, as in the reference implementation.
    pub mean: f32,
    pub std: f32,
    pub n_classes: usize,
    pub timestep: usize,
    pub weights: String,
}

/// Frame-wise log-probabilities on the global `hop`-spaced grid.
#[derive(Clone, Debug)]
pub struct ChordPosteriors {
    pub frames: usize,
    /// Row-major `[frames, 170]`, natural-log probabilities.
    pub log_probs: Vec<f32>,
    /// Seconds between frames (`hop / sample_rate`).
    pub frame_secs: f64,
}

impl ChordPosteriors {
    pub fn row(&self, t: usize) -> &[f32] {
        &self.log_probs[t * N_CLASSES..(t + 1) * N_CLASSES]
    }

    pub fn argmax(&self, t: usize) -> u16 {
        let row = self.row(t);
        let mut best = 0;
        for (i, v) in row.iter().enumerate() {
            if *v > row[best] {
                best = i;
            }
        }
        best as u16
    }
}

pub struct ChordModel {
    session: Session,
    meta: BtcMeta,
    cqt: Cqt,
}

impl ChordModel {
    /// Load `btc.onnx` and its `btc.json` sidecar. `threads = 0` lets ONNX Runtime decide.
    pub fn load(onnx: &Path, threads: usize) -> Result<Self> {
        Self::load_on(onnx, Device::Cpu, threads)
    }

    /// Load on a specific device (see `ortutil::Device`).
    pub fn load_on(onnx: &Path, device: Device, threads: usize) -> Result<Self> {
        let meta_path = onnx.with_extension("json");
        let meta: BtcMeta = serde_json::from_slice(
            &std::fs::read(&meta_path).with_context(|| format!("read {}", meta_path.display()))?,
        )
        .context("parse btc.json")?;
        ensure!(meta.n_classes == N_CLASSES && meta.timestep == TIMESTEP, "unexpected BTC export shape");

        let session = session(onnx, device, threads, false)?;
        Ok(Self { session, meta, cqt: Cqt::new(CqtParams::BTC) })
    }

    pub fn meta(&self) -> &BtcMeta {
        &self.meta
    }

    /// Raw model call: `features` is `[batch, 108, 144]` log-CQT, returns `[batch, 108, 170]` logits.
    pub fn logits(&mut self, features: Vec<f32>, batch: usize) -> Result<Vec<f32>> {
        let n_bins = self.cqt.params().n_bins;
        ensure!(features.len() == batch * TIMESTEP * n_bins, "bad feature length");
        let input = Tensor::from_array(([batch, TIMESTEP, n_bins], features))?;
        let outputs = self.session.run(ort::inputs!["logcqt" => input])?;
        let (_, data) = outputs["logits"].try_extract_tensor::<f32>()?;
        Ok(data.to_vec())
    }

    /// Chord posteriors for mono 22.05 kHz audio.
    pub fn analyze(&mut self, y: &[f32]) -> Result<ChordPosteriors> {
        let p = *self.cqt.params();
        let n_bins = p.n_bins;
        let frames = self.cqt.frames_for(y.len());
        let step = WINDOW_HOP_FRAMES * p.hop;

        // Instance starts; drop a trailing instance shorter than one hop (no new frames).
        let mut starts: Vec<usize> = (0..).map(|w| w * step).take_while(|s| *s < y.len().max(1)).collect();
        if starts.len() > 1 && y.len() - starts[starts.len() - 1] < p.hop {
            starts.pop();
        }

        let batch = starts.len();
        // Every instance has its own CQT, so they are independent: spread them over a few
        // threads (the CQT, not the network, is what takes time here).
        let mut features = vec![self.meta.mean; batch * TIMESTEP * n_bins];
        let mut valid = vec![0usize; batch];
        let workers = std::thread::available_parallelism().map(|n| n.get() / 2).unwrap_or(2).clamp(1, 6).min(batch.max(1));
        let per_worker = batch.div_ceil(workers).max(1);
        let cqt = &self.cqt;
        std::thread::scope(|scope| {
            let jobs = features
                .chunks_mut(per_worker * TIMESTEP * n_bins)
                .zip(valid.chunks_mut(per_worker))
                .zip(starts.chunks(per_worker));
            for ((feature_rows, valid_rows), window_starts) in jobs {
                scope.spawn(move || {
                    for (k, &s) in window_starts.iter().enumerate() {
                        let end = (s + WINDOW_SAMPLES).min(y.len());
                        let feat = cqt.log_magnitude(&y[s..end], PadMode::Reflect);
                        let got = (feat.len() / n_bins).min(TIMESTEP);
                        let dst = k * TIMESTEP * n_bins;
                        feature_rows[dst..dst + got * n_bins].copy_from_slice(&feat[..got * n_bins]);
                        valid_rows[k] = got;
                    }
                });
            }
        });

        // Bounded batches keep peak memory flat on long files.
        const MAX_BATCH: usize = 16;
        let mut acc = vec![0f32; frames * N_CLASSES];
        let mut wsum = vec![0f32; frames];
        let mut row = vec![0f32; N_CLASSES];
        for chunk_start in (0..batch).step_by(MAX_BATCH) {
            let chunk_end = (chunk_start + MAX_BATCH).min(batch);
            let b = chunk_end - chunk_start;
            let slice = features[chunk_start * TIMESTEP * n_bins..chunk_end * TIMESTEP * n_bins].to_vec();
            let logits = self.logits(slice, b)?;
            for w in chunk_start..chunk_end {
                let base = (w - chunk_start) * TIMESTEP * N_CLASSES;
                let first = w == 0;
                let last = w + 1 == batch;
                for i in 0..valid[w] {
                    let t = w * WINDOW_HOP_FRAMES + i;
                    if t >= frames {
                        break;
                    }
                    // Triangular weight, except that the outer halves of the first and last
                    // instance have no partner and keep full weight.
                    let tri = (i + 1).min(TIMESTEP - i) as f32;
                    let weight = if (first && i < TIMESTEP / 2) || (last && i >= TIMESTEP / 2) {
                        (TIMESTEP / 2) as f32
                    } else {
                        tri
                    };
                    log_softmax(&logits[base + i * N_CLASSES..base + (i + 1) * N_CLASSES], &mut row);
                    let dst = &mut acc[t * N_CLASSES..(t + 1) * N_CLASSES];
                    for (d, r) in dst.iter_mut().zip(&row) {
                        *d += weight * r;
                    }
                    wsum[t] += weight;
                }
            }
        }
        for t in 0..frames {
            let dst = &mut acc[t * N_CLASSES..(t + 1) * N_CLASSES];
            if wsum[t] > 0.0 {
                let inv = 1.0 / wsum[t];
                dst.iter_mut().for_each(|v| *v *= inv);
                // Renormalise: a weighted geometric mean of distributions does not sum to one.
                let src = dst.to_vec();
                log_softmax(&src, dst);
            } else {
                dst.fill(-(N_CLASSES as f32).ln());
            }
        }
        Ok(ChordPosteriors { frames, log_probs: acc, frame_secs: p.hop as f64 / p.sample_rate as f64 })
    }
}

fn log_softmax(logits: &[f32], out: &mut [f32]) {
    let max = logits.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
    let lse = max + logits.iter().map(|v| (v - max).exp()).sum::<f32>().ln();
    for (o, v) in out.iter_mut().zip(logits) {
        *o = v - lse;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_softmax_normalises() {
        let mut out = [0f32; 4];
        log_softmax(&[1.0, 2.0, 3.0, 4.0], &mut out);
        let total: f32 = out.iter().map(|v| v.exp()).sum();
        assert!((total - 1.0).abs() < 1e-6);
        assert!(out[3] > out[2]);
    }
}
