//! Parity tests against references produced once by tools/export/make_golden.py
//! (librosa 0.11 + PyTorch). Audio is synthetic, so the fixtures are committed.

use std::path::{Path, PathBuf};

use chordku_core::btc::{ChordModel, TIMESTEP};
use chordku_core::chords::N_CLASSES;
use chordku_core::cqt::{Cqt, CqtParams, PadMode};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap()
}

fn load_f32(name: &str) -> Vec<f32> {
    let bytes = std::fs::read(root().join("testdata/golden").join(name)).expect("golden file");
    bytes.chunks_exact(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect()
}

fn model_path() -> Option<PathBuf> {
    let p = std::env::var_os("CHORDKU_MODELS").map(PathBuf::from).unwrap_or_else(|| root().join("models"));
    let onnx = p.join("btc.onnx");
    onnx.exists().then_some(onnx)
}

#[test]
fn cqt_matches_numpy_direct_and_librosa() {
    let y = load_f32("synth_22k.f32");
    let direct = load_f32("synth_logcqt_dir.f32");
    let librosa = load_f32("synth_logcqt_lib.f32");
    let cqt = Cqt::new(CqtParams::BTC);
    let got = cqt.log_magnitude(&y[..220_500], PadMode::Reflect);
    assert_eq!(got.len(), TIMESTEP * 144);

    // Same algorithm in numpy (f64 basis, untruncated spectrum). The Rust plan keeps only the
    // band around each wavelet and runs the FFT in f32, so the tolerance loosens towards the
    // noise floor: bins 35 dB below the peaks may differ slightly, bins that matter may not.
    for (floor, tol) in [(-2.0f32, 0.005f32), (-4.0, 0.05)] {
        let errs: Vec<f32> =
            got.iter().zip(&direct).filter(|(_, d)| **d > floor).map(|(g, d)| (g - d).abs()).collect();
        assert!(errs.len() > 2000, "fixture has too little energy above {floor}");
        let worst = errs.iter().cloned().fold(0f32, f32::max);
        assert!(worst < tol, "max |rust - numpy direct| = {worst} above {floor} ({} bins)", errs.len());
    }

    // librosa's recursive CQT: statistically identical on bins that carry the music.
    let diffs: Vec<f32> = got.iter().zip(&librosa).filter(|(_, l)| **l > -2.0).map(|(g, l)| g - l).collect();
    let mean = diffs.iter().sum::<f32>() / diffs.len() as f32;
    let std = (diffs.iter().map(|d| (d - mean).powi(2)).sum::<f32>() / diffs.len() as f32).sqrt();
    assert!(mean.abs() < 0.005 && std < 0.02, "vs librosa: mean {mean} std {std} over {} bins", diffs.len());
}

#[test]
fn btc_logits_match_pytorch() {
    let Some(onnx) = model_path() else {
        eprintln!("skipped: models/btc.onnx not found (run tools/export/export_btc.py)");
        return;
    };
    let mut model = ChordModel::load(&onnx, 1).unwrap();
    let features = load_f32("synth_logcqt_lib.f32");
    let want = load_f32("synth_logits.f32");
    let got = model.logits(features, 1).unwrap();
    assert_eq!(got.len(), TIMESTEP * N_CLASSES);
    let worst = got.iter().zip(&want).map(|(a, b)| (a - b).abs()).fold(0f32, f32::max);
    assert!(worst < 1e-3, "max |onnx - pytorch| logit = {worst}");
}

#[test]
fn recognises_the_synthetic_progression() {
    let Some(onnx) = model_path() else {
        eprintln!("skipped: models/btc.onnx not found");
        return;
    };
    let mut model = ChordModel::load(&onnx, 1).unwrap();
    let y = load_f32("synth_22k.f32");
    let post = model.analyze(&y).unwrap();
    assert_eq!(post.frames, 1 + y.len() / 2048);

    // C, A:min, F, G:7 in 2.5 s bars, repeated twice over 20 s.
    let expected = [1u16, 9 * 14, 5 * 14 + 1, 7 * 14 + 9];
    for bar in 0..8 {
        let lo = ((bar as f64 * 2.5 + 0.4) / post.frame_secs) as usize;
        let hi = (((bar + 1) as f64 * 2.5 - 0.4) / post.frame_secs) as usize;
        let mut votes = std::collections::HashMap::new();
        for t in lo..hi.min(post.frames) {
            *votes.entry(post.argmax(t)).or_insert(0usize) += 1;
        }
        let top = votes.into_iter().max_by_key(|(_, c)| *c).unwrap().0;
        assert_eq!(top, expected[bar % 4], "bar {bar}");
    }
}

#[test]
fn tracks_a_synthetic_drum_pattern() {
    use chordku_core::beats::BeatTracker;
    let models = model_path().map(|p| p.parent().unwrap().to_path_buf());
    let Some(models) = models.filter(|m| m.join("beat_this_small.onnx").exists()) else {
        eprintln!("skipped: beat models not found");
        return;
    };
    // 100 BPM, 4/4, 24 s: kick on 1 and 3, snare on 2 and 4, hats on eighths, bass note per bar.
    let sr = 22_050usize;
    let beat = 0.6f64;
    let mut y = vec![0f32; sr * 24];
    let mut seed = 0x2545_F491u32;
    let mut noise = move || {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        (seed as f32 / u32::MAX as f32) * 2.0 - 1.0
    };
    for eighth in 0..(24.0 / (beat / 2.0)) as usize {
        let t0 = (eighth as f64 * beat / 2.0 * sr as f64) as usize;
        let on_beat = eighth % 2 == 0;
        let beat_no = (eighth / 2) % 4;
        for i in 0..(sr / 8).min(y.len().saturating_sub(t0)) {
            let t = i as f32 / sr as f32;
            let mut v = noise() * 0.08 * (-t * 60.0).exp(); // hat
            if on_beat && beat_no % 2 == 0 {
                v += 0.9 * (2.0 * std::f32::consts::PI * (55.0 + 80.0 * (-t * 30.0).exp()) * t).sin() * (-t * 18.0).exp();
            }
            if on_beat && beat_no % 2 == 1 {
                v += 0.5 * noise() * (-t * 25.0).exp();
            }
            if on_beat && beat_no == 0 {
                v += 0.3 * (2.0 * std::f32::consts::PI * 110.0 * t).sin() * (-t * 4.0).exp();
            }
            y[t0 + i] += v;
        }
    }
    let mut tracker =
        BeatTracker::load(&models.join("mel_spectrogram.onnx"), &models.join("beat_this_small.onnx"), 1).unwrap();
    let grid = tracker.analyze(&y).unwrap();
    let bpm = grid.bpm().expect("tempo");
    assert!((bpm - 100.0).abs() < 1.5, "bpm {bpm}");
    // Every detected beat must sit on the 0.6 s grid (within 40 ms).
    for b in &grid.beats {
        let phase = (b / beat).round() * beat;
        assert!((b - phase).abs() < 0.04, "beat at {b:.3} is off the grid");
    }
    assert!(grid.beats.len() >= 36, "only {} beats", grid.beats.len());
    assert_eq!(grid.beats_per_bar(), Some(4));
}
