//! The fast analysis path: PCM in, chart out, in a few seconds.
//!
//!   mono 22.05 kHz -> tuning estimate -> beats            (on the untouched audio)
//!                                     -> chords           (on the tuning-corrected audio)
//!                                     -> chart::build
//!
//! Beat tracking and chord recognition are independent and run concurrently.

use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{Context, Result};
use serde::Serialize;

use crate::audio::Audio;
use crate::beats::{BeatTracker, BEAT_SAMPLE_RATE};
use crate::btc::ChordModel;
use crate::chart::{self, Analysis};
use crate::{resample, tuning};

pub const ANALYSIS_RATE: u32 = 22_050;

/// File names inside the models directory.
pub const BTC_ONNX: &str = "btc.onnx";
pub const BTC_META: &str = "btc.json";
pub const MEL_ONNX: &str = "mel_spectrogram.onnx";
pub const BEAT_ONNX: &str = "beat_this_small.onnx";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Stage {
    Resample,
    Tuning,
    BeatsAndChords,
    Chart,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Timings {
    pub resample_ms: u32,
    pub tuning_ms: u32,
    pub beats_ms: u32,
    pub chords_ms: u32,
    pub chart_ms: u32,
}

pub struct Output {
    pub analysis: Analysis,
    pub timings: Timings,
    pub posteriors: crate::btc::ChordPosteriors,
    /// Multiply posterior frame times by this to get real time (tuning correction).
    pub time_scale: f64,
}

pub struct Analyzer {
    chords: ChordModel,
    beats: BeatTracker,
    /// Estimate the recording's offset from A440 and correct it before chord recognition.
    /// On by default; switchable so its effect can be measured (`chordku-cli eval --no-tuning`).
    pub tuning_correction: bool,
}

impl Analyzer {
    /// Which model files are missing from `dir` (empty when ready to analyse).
    pub fn missing_models(dir: &Path) -> Vec<&'static str> {
        [BTC_ONNX, BTC_META, MEL_ONNX, BEAT_ONNX].into_iter().filter(|f| !dir.join(f).exists()).collect()
    }

    /// `threads` is the ONNX Runtime intra-op thread count per model (0 = automatic).
    pub fn load(dir: &Path, threads: usize) -> Result<Self> {
        let p = |f: &str| -> PathBuf { dir.join(f) };
        Self::load_from(&p(BTC_ONNX), &p(MEL_ONNX), &p(BEAT_ONNX), threads)
    }

    /// Load from explicit files (`btc.json` must sit next to `btc_onnx`).
    pub fn load_from(btc_onnx: &Path, mel_onnx: &Path, beat_onnx: &Path, threads: usize) -> Result<Self> {
        Self::load_on(btc_onnx, mel_onnx, beat_onnx, crate::accel::Plan::default(), threads)
    }

    /// Load each model on the device the plan names (see `accel`).
    pub fn load_on(btc_onnx: &Path, mel_onnx: &Path, beat_onnx: &Path, plan: crate::accel::Plan, threads: usize) -> Result<Self> {
        Ok(Self {
            chords: ChordModel::load_on(btc_onnx, plan.chords, threads).context("chord model")?,
            beats: BeatTracker::load_on(mel_onnx, beat_onnx, plan.beats, threads).context("beat model")?,
            tuning_correction: true,
        })
    }

    pub fn analyze(&mut self, audio: &Audio, progress: impl FnMut(Stage)) -> Result<(Analysis, Timings)> {
        let out = self.analyze_full(audio, progress)?;
        Ok((out.analysis, out.timings))
    }

    /// Like [`analyze`](Self::analyze) but also hands back the frame-wise posteriors, for
    /// evaluation and for re-running the post-processing without the network.
    pub fn analyze_full(&mut self, audio: &Audio, mut progress: impl FnMut(Stage)) -> Result<Output> {
        let mut timings = Timings::default();
        let duration = audio.duration_secs();

        progress(Stage::Resample);
        let t = Instant::now();
        let mono = audio.to_mono();
        let y = resample::resample_mono(&mono, audio.sample_rate, ANALYSIS_RATE, 0.0)?;
        timings.resample_ms = t.elapsed().as_millis() as u32;

        progress(Stage::Tuning);
        let t = Instant::now();
        let cents = if self.tuning_correction { tuning::estimate(&y, ANALYSIS_RATE).correction_cents() } else { 0.0 };
        // A sharp recording is stretched by 2^(c/1200): pitch drops onto the A440 grid and
        // analysis time runs slower than real time by the same factor.
        let stretch = 2f64.powf(cents as f64 / 1200.0);
        let tuned =
            if cents != 0.0 { Some(resample::resample_mono(&mono, audio.sample_rate, ANALYSIS_RATE, cents)?) } else { None };
        drop(mono);
        timings.tuning_ms = t.elapsed().as_millis() as u32;

        progress(Stage::BeatsAndChords);
        debug_assert_eq!(BEAT_SAMPLE_RATE, ANALYSIS_RATE);
        let chord_input: &[f32] = tuned.as_deref().unwrap_or(&y);
        let (chords_model, beat_model) = (&mut self.chords, &mut self.beats);
        let (post, grid, chords_ms, beats_ms) = std::thread::scope(|scope| {
            let chords = scope.spawn(move || {
                let t = Instant::now();
                chords_model.analyze(chord_input).map(|p| (p, t.elapsed().as_millis() as u32))
            });
            let t = Instant::now();
            let grid = beat_model.analyze(&y);
            let beats_ms = t.elapsed().as_millis() as u32;
            let chords = chords.join().map_err(|_| anyhow::anyhow!("chord thread panicked"))?;
            let (post, chords_ms) = chords?;
            anyhow::Ok((post, grid?, chords_ms, beats_ms))
        })?;
        timings.chords_ms = chords_ms;
        timings.beats_ms = beats_ms;

        progress(Stage::Chart);
        let t = Instant::now();
        let analysis = chart::build(&post, 1.0 / stretch, &grid, duration, cents);
        timings.chart_ms = t.elapsed().as_millis() as u32;
        Ok(Output { analysis, timings, posteriors: post, time_scale: 1.0 / stretch })
    }
}
