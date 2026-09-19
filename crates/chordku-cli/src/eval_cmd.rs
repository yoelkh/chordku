//! `chordku-cli eval <dir>`: score the analysis pipeline against `.lab` ground truth.
//!
//! Every `<name>.lab` next to an audio file of the same name is one test item. Two systems
//! are scored on each, so the contribution of the post-processing is visible:
//!
//!   frames  per-frame argmax of the averaged BTC posteriors (no smoothing, no beats)
//!   chart   what the user sees: tuning correction, sticky Viterbi, beat-synchronous chords

use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{bail, Context, Result};
use chordku_core::chords::Chord;
use chordku_core::eval::{self, Interval, Label, Scores};
use chordku_core::pipeline::Analyzer;
use chordku_core::decode;

const AUDIO_EXT: &[&str] = &["wav", "flac", "mp3", "m4a", "ogg", "opus", "webm"];

fn find_items(dir: &Path, out: &mut Vec<(PathBuf, PathBuf)>, depth: usize) -> Result<()> {
    for entry in std::fs::read_dir(dir).with_context(|| format!("read {}", dir.display()))? {
        let path = entry?.path();
        if path.is_dir() && depth < 3 {
            find_items(&path, out, depth + 1)?;
        } else if path.extension().and_then(|e| e.to_str()) == Some("lab") {
            if let Some(audio) = AUDIO_EXT.iter().map(|e| path.with_extension(e)).find(|p| p.is_file()) {
                out.push((audio, path));
            }
        }
    }
    Ok(())
}

pub fn run(mut args: Vec<String>, models: &Path, threads: usize, take_opt: fn(&mut Vec<String>, &str) -> Option<String>) -> Result<()> {
    let limit: usize = take_opt(&mut args, "--limit").map(|v| v.parse()).transpose()?.unwrap_or(usize::MAX);
    let json = take_opt(&mut args, "--json");
    let no_tuning = args.iter().position(|a| a == "--no-tuning").map(|i| args.remove(i)).is_some();
    let tuning_correction = !no_tuning;
    let detune_cents: f64 = take_opt(&mut args, "--detune").map(|v| v.parse()).transpose()?.unwrap_or(0.0);
    let tuning_only = args.iter().position(|a| a == "--tuning-only").map(|i| args.remove(i)).is_some();
    let dir =PathBuf::from(args.first().context("missing <dir> with audio + .lab pairs")?);

    let mut items = Vec::new();
    find_items(&dir, &mut items, 0)?;
    items.sort();
    items.truncate(limit);
    if items.is_empty() {
        bail!("no `<name>.lab` + audio pairs under {}", dir.display());
    }

    if tuning_only {
        // Estimator study: the same recordings, peaks up to 2 kHz / 1 kHz / 500 Hz.
        println!("file\tcents<2k\tconf\tcents<1k\tconf\tcents<500\tconf");
        for (audio_path, _) in &items {
            let audio = decode::decode_file(audio_path)?;
            let y = chordku_core::resample::resample_mono(&audio.to_mono(), audio.sample_rate, 22_050, 0.0)?;
            let row: Vec<String> = [2000.0f32, 1000.0, 500.0]
                .iter()
                .map(|hi| {
                    let t = chordku_core::tuning::estimate_in_band(&y, 22_050, 80.0, *hi);
                    format!("{:+.1}\t{:.2}", t.cents, t.confidence)
                })
                .collect();
            println!("{}\t{}", audio_path.file_stem().and_then(|s| s.to_str()).unwrap_or("?"), row.join("\t"));
        }
        return Ok(());
    }

    let mut analyzer = Analyzer::load(models, threads)?;
    analyzer.tuning_correction = tuning_correction;
    let (mut frames_all, mut chart_all) = (Vec::new(), Vec::new());
    let mut rows = Vec::new();
    let (mut audio_secs, started) = (0.0, Instant::now());
    for (i, (audio_path, lab_path)) in items.iter().enumerate() {
        let mut reference = eval::read_lab(lab_path)?;
        let mut audio = decode::decode_file(audio_path)?;
        if detune_cents != 0.0 {
            // Controlled mistuning: resampling by 2^(-c/1200) and playing at the original rate
            // raises the pitch by c cents and shortens the recording by the same factor.
            let ratio = 2f64.powf(-detune_cents / 1200.0);
            audio.samples = chordku_core::resample::resample_interleaved(&audio.samples, audio.channels, ratio)?;
            for iv in &mut reference {
                iv.t0 *= ratio;
                iv.t1 *= ratio;
            }
        }
        audio_secs += audio.duration_secs();
        let out = analyzer.analyze_full(&audio, |_| {})?;

        let step = out.posteriors.frame_secs * out.time_scale;
        let frames: Vec<Interval> = (0..out.posteriors.frames)
            .map(|t| Interval {
                t0: (t as f64 - 0.5).max(0.0) * step,
                t1: (t as f64 + 0.5) * step,
                label: Label::from(Chord::from_class(out.posteriors.argmax(t))),
            })
            .collect();
        let chart: Vec<Interval> = out
            .analysis
            .segments
            .iter()
            .map(|s| Interval { t0: s.t0_ms as f64 / 1e3, t1: s.t1_ms as f64 / 1e3, label: Label::from(Chord::from_class(s.class)) })
            .collect();
        let (f, c) = (eval::score(&reference, &frames), eval::score(&reference, &chart));
        eprintln!(
            "[{}/{}] {:<44} frames majmin {:5.1}  chart majmin {:5.1}  root {:5.1}  7ths {:5.1}",
            i + 1,
            items.len(),
            audio_path.file_stem().and_then(|s| s.to_str()).unwrap_or("?").chars().take(44).collect::<String>(),
            f.majmin * 100.0,
            c.majmin * 100.0,
            c.root * 100.0,
            c.sevenths * 100.0
        );
        rows.push(serde_json::json!({
            "file": audio_path.file_name().and_then(|s| s.to_str()),
            "frames": {"root": f.root, "majmin": f.majmin, "sevenths": f.sevenths},
            "chart": {"root": c.root, "majmin": c.majmin, "sevenths": c.sevenths},
            "tuning_cents": out.analysis.tuning_cents,
            "bpm": out.analysis.bpm,
        }));
        frames_all.push(f);
        chart_all.push(c);
    }

    let (f, c) = (eval::weighted_mean(&frames_all), eval::weighted_mean(&chart_all));
    let line = |name: &str, s: &Scores| println!("{name:<8} root {:5.2}   majmin {:5.2}   sevenths {:5.2}", s.root * 100.0, s.majmin * 100.0, s.sevenths * 100.0);
    println!("WCSR over {} recordings, {:.0} min of audio ({:.0} s of compute)", items.len(), audio_secs / 60.0, started.elapsed().as_secs_f64());
    line("frames", &f);
    line("chart", &c);
    if let Some(path) = json {
        let summary = serde_json::json!({
            "items": rows,
            "frames": {"root": f.root, "majmin": f.majmin, "sevenths": f.sevenths},
            "chart": {"root": c.root, "majmin": c.majmin, "sevenths": c.sevenths},
            "audio_minutes": audio_secs / 60.0,
        });
        std::fs::write(&path, serde_json::to_vec_pretty(&summary)?).with_context(|| format!("write {path}"))?;
    }
    Ok(())
}
