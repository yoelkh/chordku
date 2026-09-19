//! ChordKu command line: the same analysis core as the server, for testing and benchmarks.

mod bench_cmd;
mod eval_cmd;

use std::path::{Path, PathBuf};
use std::time::Instant;

use anyhow::{bail, Context, Result};
use chordku_core::beats::BeatTracker;
use chordku_core::btc::ChordModel;
use chordku_core::chords::Chord;
use chordku_core::pipeline::Analyzer;
use chordku_core::{decode, resample};

const USAGE: &str = "\
chordku-cli <command> [options]

  analyze <audio> [--models DIR] [--threads N] [--json FILE]  full chart: key, tempo, bars, sections
  chords <audio> [--models DIR] [--threads N] [--lab FILE]   frame-wise chord labels (.lab)
  beats  <audio> [--models DIR] [--threads N]                 beat / downbeat times, tempo, meter
  separate <audio> <out-dir> [--models DIR] [--threads N]     drums/bass/other/vocals WAVs (htdemucs)
  eval <dir> [--models DIR] [--limit N] [--json FILE]         WCSR against <name>.lab ground truth
  bench <audio> [--devices cpu,gpu_high,gpu_low,npu] [--only chords,beats,stems] [--json FILE]
                                                              speed and fidelity of every model on every device
  info                                                        ONNX Runtime build info
";

fn main() -> Result<()> {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        print!("{USAGE}");
        return Ok(());
    }
    let cmd = args.remove(0);
    match cmd.as_str() {
        "info" => {
            println!("{}", chordku_core::ort_version());
            Ok(())
        }
        "analyze" => analyze(args),
        "chords" => chords(args),
        "beats" => beats(args),
        "separate" => separate(args),
        "eval" => {
            let mut args = args;
            let models = models_dir(&mut args);
            let threads: usize = take_opt(&mut args, "--threads").map(|t| t.parse()).transpose()?.unwrap_or(0);
            eval_cmd::run(args, &models, threads, take_opt)
        }
        "bench" => {
            let mut args = args;
            let models = models_dir(&mut args);
            let threads: usize = take_opt(&mut args, "--threads").map(|t| t.parse()).transpose()?.unwrap_or(0);
            bench_cmd::run(args, &models, threads, take_opt)
        }
        _ => {
            print!("{USAGE}");
            bail!("unknown command `{cmd}`")
        }
    }
}

fn take_opt(args: &mut Vec<String>, name: &str) -> Option<String> {
    let i = args.iter().position(|a| a == name)?;
    if i + 1 >= args.len() {
        return None;
    }
    args.remove(i);
    Some(args.remove(i))
}

fn models_dir(args: &mut Vec<String>) -> PathBuf {
    take_opt(args, "--models")
        .or_else(|| std::env::var("CHORDKU_MODELS").ok())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("models"))
}

fn chords(mut args: Vec<String>) -> Result<()> {
    let models = models_dir(&mut args);
    let threads: usize = take_opt(&mut args, "--threads").map(|t| t.parse()).transpose()?.unwrap_or(0);
    let lab = take_opt(&mut args, "--lab");
    let audio_path = args.first().context("missing <audio>")?;

    let t0 = Instant::now();
    let audio = decode::decode_file(Path::new(audio_path))?;
    let t_decode = t0.elapsed();
    let mono = audio.to_mono();
    let y = resample::resample_mono(&mono, audio.sample_rate, 22_050, 0.0)?;
    let t_resample = t0.elapsed() - t_decode;

    let mut model = ChordModel::load(&models.join("btc.onnx"), threads)?;
    let t_load = t0.elapsed() - t_decode - t_resample;
    let post = model.analyze(&y)?;
    let t_chords = t0.elapsed() - t_decode - t_resample - t_load;

    // Collapse equal neighbouring frames into .lab segments.
    let mut out = String::new();
    let mut start = 0usize;
    for t in 1..=post.frames {
        if t == post.frames || post.argmax(t) != post.argmax(start) {
            out.push_str(&format!(
                "{:.3}\t{:.3}\t{}\n",
                start as f64 * post.frame_secs,
                (t as f64 * post.frame_secs).min(audio.duration_secs()),
                Chord::from_class(post.argmax(start)).harte()
            ));
            start = t;
        }
    }
    match lab {
        Some(p) => std::fs::write(&p, &out).with_context(|| format!("write {p}"))?,
        None => print!("{out}"),
    }
    eprintln!(
        "{:.1}s audio | decode {:.2}s | resample {:.2}s | model load {:.2}s | cqt+btc {:.2}s | total {:.2}s",
        audio.duration_secs(),
        t_decode.as_secs_f64(),
        t_resample.as_secs_f64(),
        t_load.as_secs_f64(),
        t_chords.as_secs_f64(),
        t0.elapsed().as_secs_f64()
    );
    Ok(())
}

fn beats(mut args: Vec<String>) -> Result<()> {
    let models = models_dir(&mut args);
    let threads: usize = take_opt(&mut args, "--threads").map(|t| t.parse()).transpose()?.unwrap_or(0);
    let audio_path = args.first().context("missing <audio>")?;

    let t0 = Instant::now();
    let audio = decode::decode_file(Path::new(audio_path))?;
    let y = resample::resample_mono(&audio.to_mono(), audio.sample_rate, 22_050, 0.0)?;
    let t_prep = t0.elapsed();
    let mut tracker =
        BeatTracker::load(&models.join("mel_spectrogram.onnx"), &models.join("beat_this_small.onnx"), threads)?;
    let t_load = t0.elapsed() - t_prep;
    let grid = tracker.analyze(&y)?;
    let t_beats = t0.elapsed() - t_prep - t_load;

    for b in &grid.beats {
        let down = grid.downbeats.iter().any(|d| (d - b).abs() < 1e-6);
        println!("{b:.3}	{}", if down { 1 } else { 0 });
    }
    eprintln!(
        "{:.1}s audio | {} beats, {} downbeats | {:.1} BPM | {} beats/bar | prep {:.2}s | load {:.2}s | beats {:.2}s",
        audio.duration_secs(),
        grid.beats.len(),
        grid.downbeats.len(),
        grid.bpm().unwrap_or(0.0),
        grid.beats_per_bar().map(|n| n.to_string()).unwrap_or_else(|| "?".into()),
        t_prep.as_secs_f64(),
        t_load.as_secs_f64(),
        t_beats.as_secs_f64()
    );
    Ok(())
}

fn analyze(mut args: Vec<String>) -> Result<()> {
    let models = models_dir(&mut args);
    let threads: usize = take_opt(&mut args, "--threads").map(|t| t.parse()).transpose()?.unwrap_or(0);
    let json = take_opt(&mut args, "--json");
    let audio_path = args.first().context("missing <audio>")?;
    let missing = Analyzer::missing_models(&models);
    if !missing.is_empty() {
        bail!("missing model files in {}: {}", models.display(), missing.join(", "));
    }

    let t0 = Instant::now();
    let audio = decode::decode_file(Path::new(audio_path))?;
    let t_decode = t0.elapsed();
    let mut analyzer = Analyzer::load(&models, threads)?;
    let t_load = t0.elapsed() - t_decode;
    let (a, timings) = analyzer.analyze(&audio, |_| {})?;

    if let Some(path) = &json {
        std::fs::write(path, serde_json::to_vec_pretty(&a)?).with_context(|| format!("write {path}"))?;
    }

    const NOTES: [&str; 12] = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];
    let key = a
        .key
        .map(|k| format!("{} {:?} ({:.0}%)", NOTES[k.tonic_pc as usize], k.mode, k.confidence * 100.0))
        .unwrap_or_else(|| "?".into());
    println!(
        "key {key} | {} BPM | {}/4 | tuning {:+.0} cents | {} chords | {} sections",
        a.bpm.map(|b| format!("{b:.1}")).unwrap_or_else(|| "?".into()),
        a.meter,
        a.tuning_cents,
        a.segments.len(),
        a.sections.len()
    );
    for k in a.key_segments.iter().skip(1) {
        println!("  modulation at {:.1}s -> {} {:?}", k.t0_ms as f64 / 1e3, NOTES[k.tonic_pc as usize], k.mode);
    }
    // One line per section, bars separated by |, a chord printed where it starts.
    let bar_starts: Vec<u32> = a.downbeat_idx.iter().map(|i| a.beats_ms[*i as usize]).collect();
    for sec in &a.sections {
        let mut line = format!("[{} {:>5.1}s] ", sec.label, sec.t0_ms as f64 / 1e3);
        let mut bars = 0;
        for w in bar_starts.windows(2).filter(|w| w[0] >= sec.t0_ms && w[0] < sec.t1_ms) {
            let names: Vec<String> = a
                .segments
                .iter()
                .filter(|s| s.t1_ms > w[0] && s.t0_ms < w[1])
                .map(|s| Chord::from_class(s.class).harte())
                .collect();
            line.push_str(&format!("| {} ", names.join(" ")));
            bars += 1;
            if bars % 8 == 0 {
                line.push_str("|
            ");
            }
        }
        println!("{}|", line.trim_end());
    }
    eprintln!(
        "{:.1}s audio | decode {:.2}s | load {:.2}s | resample {} ms | tuning {} ms | beats {} ms | chords {} ms | chart {} ms | total {:.2}s",
        audio.duration_secs(),
        t_decode.as_secs_f64(),
        t_load.as_secs_f64(),
        timings.resample_ms,
        timings.tuning_ms,
        timings.beats_ms,
        timings.chords_ms,
        timings.chart_ms,
        t0.elapsed().as_secs_f64()
    );
    Ok(())
}

fn separate(mut args: Vec<String>) -> Result<()> {
    use chordku_core::separate::htdemucs::HtDemucs;
    use chordku_core::separate::{Separator, Stem, STEM_RATE};
    use std::ops::ControlFlow;

    let models = models_dir(&mut args);
    let threads: usize = take_opt(&mut args, "--threads").map(|t| t.parse()).transpose()?.unwrap_or(0);
    let max_secs: Option<f64> = take_opt(&mut args, "--seconds").map(|t| t.parse()).transpose()?;
    let audio_path = args.first().context("missing <audio>")?.clone();
    let out_dir = PathBuf::from(args.get(1).context("missing <out-dir>")?);
    std::fs::create_dir_all(&out_dir)?;

    let t0 = Instant::now();
    let audio = decode::decode_file(Path::new(&audio_path))?;
    let ratio = STEM_RATE as f64 / audio.sample_rate as f64;
    let stereo = chordku_core::Audio {
        sample_rate: audio.sample_rate,
        channels: 2,
        samples: {
            let [l, r] = audio.to_stereo_planar();
            l.iter().zip(&r).flat_map(|(a, b)| [*a, *b]).collect()
        },
    };
    let resampled = resample::resample_interleaved(&stereo.samples, 2, ratio)?;
    let mut mix = [Vec::new(), Vec::new()];
    for f in resampled.chunks_exact(2) {
        mix[0].push(f[0]);
        mix[1].push(f[1]);
    }
    if let Some(secs) = max_secs {
        let n = ((secs * STEM_RATE as f64) as usize).min(mix[0].len());
        mix[0].truncate(n);
        mix[1].truncate(n);
    }
    let secs = mix[0].len() as f64 / STEM_RATE as f64;
    let t_prep = t0.elapsed();

    let mut model = HtDemucs::load(&models.join("htdemucs_fwd.onnx"), threads)?;
    let t_load = t0.elapsed() - t_prep;

    let spec = hound::WavSpec { channels: 2, sample_rate: STEM_RATE, bits_per_sample: 32, sample_format: hound::SampleFormat::Float };
    let mut writers = Vec::new();
    for stem in Stem::ALL {
        writers.push(hound::WavWriter::create(out_dir.join(format!("{}.wav", stem.name())), spec)?);
    }
    let t_sep = Instant::now();
    model.separate(&mix, &mut |block| {
        for (w, pair) in writers.iter_mut().zip(block.stems) {
            for (l, r) in pair[0].iter().zip(&pair[1]) {
                w.write_sample(*l)?;
                w.write_sample(*r)?;
            }
        }
        eprint!("\rsegment {}/{}  ", block.segment, block.segments);
        Ok(ControlFlow::Continue(()))
    })?;
    for w in writers {
        w.finalize()?;
    }
    let sep = t_sep.elapsed().as_secs_f64();
    eprintln!(
        "\n{secs:.1}s audio | prep {:.2}s | load {:.2}s | separate {sep:.1}s (RTF {:.2}) | peak RSS {} MB",
        t_prep.as_secs_f64(),
        t_load.as_secs_f64(),
        sep / secs,
        peak_rss_mb()
    );
    Ok(())
}

/// Peak working set of this process in MB (Windows), 0 elsewhere.
fn peak_rss_mb() -> u64 {
    #[cfg(windows)]
    {
        #[repr(C)]
        #[derive(Default)]
        struct Counters {
            cb: u32,
            page_fault_count: u32,
            peak_working_set_size: usize,
            working_set_size: usize,
            quota_peak_paged: usize,
            quota_paged: usize,
            quota_peak_non_paged: usize,
            quota_non_paged: usize,
            pagefile_usage: usize,
            peak_pagefile_usage: usize,
        }
        #[link(name = "kernel32")]
        extern "system" {
            fn GetCurrentProcess() -> isize;
            fn K32GetProcessMemoryInfo(process: isize, counters: *mut Counters, cb: u32) -> i32;
        }
        let mut c = Counters { cb: std::mem::size_of::<Counters>() as u32, ..Default::default() };
        // SAFETY: plain Win32 call writing into a correctly sized, owned struct.
        let ok = unsafe { K32GetProcessMemoryInfo(GetCurrentProcess(), &mut c, c.cb) };
        if ok != 0 {
            return (c.peak_working_set_size / (1024 * 1024)) as u64;
        }
    }
    0
}
