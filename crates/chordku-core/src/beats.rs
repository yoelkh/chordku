//! Beat and downbeat tracking with "Beat This!" (Foscarin, Schlueter, Widmer, ISMIR 2024).
//!
//! Model and weights: MIT, https://github.com/CPJKU/beat_this. The ONNX graphs (log-mel
//! front end + small transformer) and the chunking / peak-picking below follow the
//! MIT-licensed Rust port https://github.com/danigb/beat-this-rs, re-implemented here so
//! that the whole application shares a single ONNX Runtime instead of bundling a second
//! inference engine.
//!
//! Pipeline: 22.05 kHz mono -> 128-band log-mel at 50 fps (inside `mel_spectrogram.onnx`)
//! -> 1500-frame chunks with a 6-frame border -> beat / downbeat logits -> peaks above 0
//! within +-3 frames -> downbeats snapped to the nearest beat. No DBN is needed.

use std::path::Path;

use anyhow::{anyhow, ensure, Result};
use ort::session::Session;
use ort::value::Tensor;
use serde::{Deserialize, Serialize};

use crate::ortutil::{session, Device};

pub const BEAT_SAMPLE_RATE: u32 = 22_050;
const FPS: f64 = 50.0;
const N_MELS: usize = 128;
const CHUNK: usize = 1500;
const BORDER: usize = 6;
const STRIDE: usize = CHUNK - 2 * BORDER;

/// Beat grid of a recording, in seconds.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct BeatGrid {
    pub beats: Vec<f64>,
    /// Subset of `beats` that start a bar.
    pub downbeats: Vec<f64>,
}

impl BeatGrid {
    /// Median tempo in beats per minute, if there are enough beats.
    pub fn bpm(&self) -> Option<f64> {
        if self.beats.len() < 4 {
            return None;
        }
        let mut ibi: Vec<f64> = self.beats.windows(2).map(|w| w[1] - w[0]).collect();
        ibi.sort_by(|a, b| a.total_cmp(b));
        let med = ibi[ibi.len() / 2];
        (med > 0.0).then(|| 60.0 / med)
    }

    /// Most common number of beats between consecutive downbeats (the meter numerator).
    pub fn beats_per_bar(&self) -> Option<usize> {
        let mut counts = [0usize; 13];
        for w in self.downbeats.windows(2) {
            let n = self.beats.iter().filter(|b| **b >= w[0] - 1e-6 && **b < w[1] - 1e-6).count();
            if (2..=12).contains(&n) {
                counts[n] += 1;
            }
        }
        let (n, c) = counts.iter().enumerate().max_by_key(|(_, c)| **c)?;
        (*c > 0).then_some(n)
    }
}

pub struct BeatTracker {
    mel: Session,
    beat: Session,
}

impl BeatTracker {
    pub fn load(mel_onnx: &Path, beat_onnx: &Path, threads: usize) -> Result<Self> {
        Self::load_on(mel_onnx, beat_onnx, Device::Cpu, threads)
    }

    /// The transformer runs on `device`. The tiny log-mel front end stays on the CPU: its
    /// input length changes with every song, which accelerators recompile for.
    pub fn load_on(mel_onnx: &Path, beat_onnx: &Path, device: Device, threads: usize) -> Result<Self> {
        Ok(Self { mel: session(mel_onnx, Device::Cpu, threads, false)?, beat: session(beat_onnx, device, threads, false)? })
    }

    /// Track beats in mono audio sampled at [`BEAT_SAMPLE_RATE`].
    pub fn analyze(&mut self, y: &[f32]) -> Result<BeatGrid> {
        if y.len() < BEAT_SAMPLE_RATE as usize / 2 {
            return Ok(BeatGrid::default());
        }
        let (mel, frames) = self.log_mel(y)?;
        let (beat, down) = self.logits(&mel, frames)?;
        let beats: Vec<f64> = find_peaks(&beat).into_iter().map(|f| f / FPS).collect();
        let mut downbeats: Vec<f64> = find_peaks(&down).into_iter().map(|f| f / FPS).collect();
        snap_to(&beats, &mut downbeats);
        Ok(BeatGrid { beats, downbeats })
    }

    fn log_mel(&mut self, y: &[f32]) -> Result<(Vec<f32>, usize)> {
        let input = Tensor::from_array(([1usize, y.len()], y.to_vec()))?;
        let outputs = self.mel.run(ort::inputs!["audio_pcm" => input])?;
        let (shape, data) = outputs["mel_spectrogram"].try_extract_tensor::<f32>()?;
        ensure!(shape.len() == 3 && shape[0] == 1 && shape[2] as usize == N_MELS, "unexpected mel shape {shape:?}");
        Ok((data.to_vec(), shape[1] as usize))
    }

    fn logits(&mut self, mel: &[f32], frames: usize) -> Result<(Vec<f32>, Vec<f32>)> {
        let mut beat = vec![-1000f32; frames];
        let mut down = vec![-1000f32; frames];
        // Later chunks first so that, where two chunks overlap, the earlier one wins
        // (the reference implementation's tie-break).
        for &start in chunk_starts(frames).iter().rev() {
            let (chunk, chunk_frames) = extract_chunk(mel, frames, start);
            let input = Tensor::from_array(([1usize, chunk_frames, N_MELS], chunk))?;
            let outputs = self.beat.run(ort::inputs!["spectrogram" => input])?;
            let b = output(&outputs, "beat", "beat_logits")?;
            let d = output(&outputs, "downbeat", "downbeat_logits")?;
            ensure!(b.len() == chunk_frames && d.len() == chunk_frames, "beat model returned a wrong length");
            let write = (start + BORDER as i64) as usize;
            for i in 0..chunk_frames - 2 * BORDER {
                if write + i < frames {
                    beat[write + i] = b[BORDER + i];
                    down[write + i] = d[BORDER + i];
                }
            }
        }
        Ok((beat, down))
    }
}

fn output(outputs: &ort::session::SessionOutputs<'_>, primary: &str, fallback: &str) -> Result<Vec<f32>> {
    let value = outputs
        .get(primary)
        .or_else(|| outputs.get(fallback))
        .ok_or_else(|| anyhow!("beat model has no `{primary}` output"))?;
    Ok(value.try_extract_tensor::<f32>()?.1.to_vec())
}

fn chunk_starts(frames: usize) -> Vec<i64> {
    let mut starts = Vec::new();
    let mut pos = -(BORDER as i64);
    while pos < frames as i64 - BORDER as i64 {
        starts.push(pos);
        pos += STRIDE as i64;
    }
    if frames > STRIDE {
        if let Some(last) = starts.last_mut() {
            *last = frames as i64 - (CHUNK - BORDER) as i64;
        }
    }
    starts
}

/// Copy `[start, start + CHUNK)` out of the spectrogram, zero-padding up to BORDER frames
/// outside the recording on either side.
fn extract_chunk(mel: &[f32], frames: usize, start: i64) -> (Vec<f32>, usize) {
    let lo = start.max(0) as usize;
    let hi = ((start + CHUNK as i64) as usize).min(frames);
    let pad_left = (-start).max(0) as usize;
    let pad_right = (start + CHUNK as i64 - frames as i64).clamp(0, BORDER as i64) as usize;
    let chunk_frames = pad_left + (hi - lo) + pad_right;
    let mut data = vec![0f32; chunk_frames * N_MELS];
    data[pad_left * N_MELS..(pad_left + hi - lo) * N_MELS].copy_from_slice(&mel[lo * N_MELS..hi * N_MELS]);
    (data, chunk_frames)
}

/// Frames whose logit is positive and the maximum within +-3 frames; adjacent peaks merge.
fn find_peaks(logits: &[f32]) -> Vec<f64> {
    let mut peaks = Vec::new();
    for i in 0..logits.len() {
        if logits[i] <= 0.0 {
            continue;
        }
        let lo = i.saturating_sub(3);
        let hi = (i + 4).min(logits.len());
        if logits[lo..hi].iter().all(|v| *v <= logits[i]) {
            peaks.push(i as f64);
        }
    }
    let mut merged: Vec<f64> = Vec::with_capacity(peaks.len());
    let mut count = 1.0;
    for p in peaks {
        match merged.last_mut() {
            Some(last) if p - *last <= 1.0 => {
                count += 1.0;
                *last += (p - *last) / count;
            }
            _ => {
                merged.push(p);
                count = 1.0;
            }
        }
    }
    merged
}

fn snap_to(beats: &[f64], downbeats: &mut Vec<f64>) {
    if beats.is_empty() {
        downbeats.clear();
        return;
    }
    for d in downbeats.iter_mut() {
        let pos = beats.partition_point(|b| *b < *d);
        let before = pos.checked_sub(1).map(|i| beats[i]);
        let after = beats.get(pos).copied();
        *d = match (before, after) {
            (Some(b), Some(a)) => {
                if (*d - b).abs() <= (a - *d).abs() {
                    b
                } else {
                    a
                }
            }
            (Some(b), None) => b,
            (None, Some(a)) => a,
            (None, None) => *d,
        };
    }
    downbeats.sort_by(|a, b| a.total_cmp(b));
    downbeats.dedup();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunk_starts_match_reference() {
        assert_eq!(chunk_starts(100), vec![-6]);
        assert_eq!(chunk_starts(1500), vec![-6, 6]);
        assert_eq!(chunk_starts(2000), vec![-6, 506]);
        assert_eq!(chunk_starts(5000), vec![-6, 1482, 2970, 3506]);
    }

    #[test]
    fn every_frame_is_covered() {
        for frames in [50usize, 100, 1488, 1500, 2000, 5000, 7800] {
            let mut seen = vec![false; frames];
            for start in chunk_starts(frames) {
                let (_, n) = extract_chunk(&vec![0.0; frames * N_MELS], frames, start);
                let write = (start + BORDER as i64) as usize;
                for i in 0..n - 2 * BORDER {
                    if write + i < frames {
                        seen[write + i] = true;
                    }
                }
            }
            assert!(seen.iter().all(|s| *s), "gap for {frames} frames");
        }
    }

    #[test]
    fn peaks_and_snapping() {
        assert_eq!(find_peaks(&[0.0, 0.0, 0.5, 1.0, 0.5, 0.0, 0.0]), vec![3.0]);
        assert_eq!(find_peaks(&[-1.0, 2.0, 2.0, -1.0]), vec![1.5]);
        let beats = [0.5, 1.0, 1.5, 2.0];
        let mut down = vec![0.52, 1.49, 1.51];
        snap_to(&beats, &mut down);
        assert_eq!(down, vec![0.5, 1.5]);
    }

    #[test]
    fn meter_and_tempo() {
        let beats: Vec<f64> = (0..32).map(|i| i as f64 * 0.5).collect();
        let downbeats: Vec<f64> = (0..8).map(|i| i as f64 * 2.0).collect();
        let grid = BeatGrid { beats, downbeats };
        assert_eq!(grid.beats_per_bar(), Some(4));
        assert!((grid.bpm().unwrap() - 120.0).abs() < 1e-9);
    }
}
