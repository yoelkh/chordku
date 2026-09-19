//! `chordku-cli bench <audio>`: which device should run which model on THIS machine?
//!
//! For every model and every device (CPU, fast GPU, power-saving GPU, NPU) this measures
//! load time, a warm run, and how far the results drift from the CPU reference. A device is
//! only eligible if it is both faster and numerically faithful: integrated GPUs are known
//! to diverge on some graphs, and a fast wrong answer is worthless.

use std::ops::ControlFlow;
use std::path::Path;
use std::time::Instant;

use anyhow::{Context, Result};
use chordku_core::beats::BeatTracker;
use chordku_core::btc::ChordModel;
use chordku_core::ortutil::Device;
use chordku_core::separate::htdemucs::{HtDemucs, SEGMENT};
use chordku_core::separate::Separator;
use chordku_core::{decode, pipeline, resample};
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct Row {
    pub model: &'static str,
    pub device: &'static str,
    pub ok: bool,
    pub load_s: f64,
    pub run_s: f64,
    /// Agreement with the CPU result: label agreement (chords), beats matched within 20 ms
    /// (beats), SNR in dB (stems).
    pub fidelity: f64,
    pub note: String,
}

fn timed<T>(f: impl FnOnce() -> Result<T>) -> Result<(T, f64)> {
    let t = Instant::now();
    let v = f()?;
    Ok((v, t.elapsed().as_secs_f64()))
}

pub fn run(mut args: Vec<String>, models: &Path, threads: usize, take_opt: fn(&mut Vec<String>, &str) -> Option<String>) -> Result<()> {
    if let Some(i) = args.iter().position(|a| a == "--calibrate") {
        // Exactly what the server does on first start, without a server.
        args.remove(i);
        let cal = chordku_core::accel::calibrate_analysis(
            &models.join(pipeline::BTC_ONNX),
            &models.join(pipeline::MEL_ONNX),
            &models.join(pipeline::BEAT_ONNX),
            threads,
            &chordku_core::accel::CANDIDATES,
        );
        let (stems, stem_trials) = chordku_core::accel::calibrate_stems(&models.join("htdemucs_fwd.onnx"), threads, &chordku_core::accel::CANDIDATES);
        for t in cal.trials.iter().chain(&stem_trials) {
            println!("{:<7} {:<9} avail={:<5} load {:>5.2}s run {:>6.2}s fidelity {:>7.3} {} {}", t.model, t.device.name(), t.available, t.load_s, t.run_s, t.fidelity, if t.accepted { "<== chosen" } else { "" }, t.note);
        }
        println!("plan: chords {}, beats {}, stems {}", cal.plan.chords.name(), cal.plan.beats.name(), stems.name());
        return Ok(());
    }
    let json = take_opt(&mut args, "--json");
    let only: Option<Vec<String>> = take_opt(&mut args, "--only").map(|s| s.split(',').map(str::to_owned).collect());
    let devices: Vec<Device> = match take_opt(&mut args, "--devices") {
        Some(list) => list.split(',').filter_map(Device::parse).collect(),
        None => Device::ALL.to_vec(),
    };
    let audio_path = args.first().context("missing <audio> (any song of a few minutes)")?;
    let wanted = |m: &str| only.as_ref().is_none_or(|o| o.iter().any(|x| x == m));

    let audio = decode::decode_file(Path::new(audio_path))?;
    let y = resample::resample_mono(&audio.to_mono(), audio.sample_rate, 22_050, 0.0)?;
    let secs = y.len() as f64 / 22_050.0;
    let mut rows: Vec<Row> = Vec::new();
    println!("{:<8} {:<9} {:>8} {:>9} {:>10}  note", "model", "device", "load s", "run s", "fidelity");

    // ---- chords ---------------------------------------------------------------------------
    if wanted("chords") {
        let mut reference: Option<Vec<u16>> = None;
        for &device in &devices {
            let row = (|| -> Result<Row> {
                let (mut model, load_s) = timed(|| ChordModel::load_on(&models.join(pipeline::BTC_ONNX), device, threads))?;
                model.analyze(&y[..y.len().min(22_050 * 20)])?; // warm-up: shader / kernel compilation
                let (post, run_s) = timed(|| model.analyze(&y))?;
                let labels: Vec<u16> = (0..post.frames).map(|t| post.argmax(t)).collect();
                let fidelity = match &reference {
                    Some(r) => r.iter().zip(&labels).filter(|(a, b)| a == b).count() as f64 / r.len().max(1) as f64 * 100.0,
                    None => 100.0,
                };
                if device == Device::Cpu {
                    reference = Some(labels);
                }
                Ok(Row { model: "chords", device: device.name(), ok: true, load_s, run_s, fidelity, note: format!("{secs:.0} s of audio, % labels equal to CPU") })
            })();
            push(&mut rows, "chords", device, row);
        }
    }

    // ---- beats ----------------------------------------------------------------------------
    if wanted("beats") {
        let mut reference: Option<Vec<f64>> = None;
        for &device in &devices {
            let row = (|| -> Result<Row> {
                let (mut model, load_s) = timed(|| {
                    BeatTracker::load_on(&models.join(pipeline::MEL_ONNX), &models.join(pipeline::BEAT_ONNX), device, threads)
                })?;
                model.analyze(&y[..y.len().min(22_050 * 35)])?;
                let (grid, run_s) = timed(|| model.analyze(&y))?;
                let fidelity = match &reference {
                    Some(r) => {
                        let hit = r.iter().filter(|b| grid.beats.iter().any(|g| (g - **b).abs() <= 0.02)).count();
                        hit as f64 / r.len().max(1) as f64 * 100.0
                    }
                    None => 100.0,
                };
                if device == Device::Cpu {
                    reference = Some(grid.beats.clone());
                }
                Ok(Row { model: "beats", device: device.name(), ok: true, load_s, run_s, fidelity, note: format!("{} beats, % of CPU beats matched", grid.beats.len()) })
            })();
            push(&mut rows, "beats", device, row);
        }
    }

    // ---- stems ----------------------------------------------------------------------------
    if wanted("stems") && models.join("htdemucs_fwd.onnx").is_file() {
        // Three segments of real audio from the middle of the song, 44.1 kHz stereo.
        let [l, r] = audio.to_stereo_planar();
        let ratio = 44_100.0 / audio.sample_rate as f64;
        let inter: Vec<f32> = l.iter().zip(&r).flat_map(|(a, b)| [*a, *b]).collect();
        let res = resample::resample_interleaved(&inter, 2, ratio)?;
        let want = SEGMENT * 3;
        let frames = res.len() / 2;
        let skip = (frames / 4).min(frames.saturating_sub(want));
        let mut mix = [Vec::new(), Vec::new()];
        for f in res.chunks_exact(2).skip(skip).take(want) {
            mix[0].push(f[0]);
            mix[1].push(f[1]);
        }
        let stem_secs = mix[0].len() as f64 / 44_100.0;
        let mut reference: Option<Vec<f32>> = None;
        for &device in &devices {
            let row = (|| -> Result<Row> {
                let (mut model, load_s) = timed(|| HtDemucs::load_on(&models.join("htdemucs_fwd.onnx"), device, threads))?;
                let short = [mix[0][..SEGMENT.min(mix[0].len())].to_vec(), mix[1][..SEGMENT.min(mix[1].len())].to_vec()];
                model.separate(&short, &mut |_| Ok(ControlFlow::Continue(())))?; // warm-up
                let mut vocals: Vec<f32> = Vec::new();
                let (_, run_s) = timed(|| {
                    model.separate(&mix, &mut |block| {
                        vocals.extend_from_slice(&block.stems[3][0]);
                        vocals.extend_from_slice(&block.stems[2][0]);
                        Ok(ControlFlow::Continue(()))
                    })
                })?;
                let fidelity = match &reference {
                    Some(r) => {
                        let sig: f64 = r.iter().map(|v| (*v as f64).powi(2)).sum();
                        let err: f64 = r.iter().zip(&vocals).map(|(a, b)| ((a - b) as f64).powi(2)).sum();
                        10.0 * (sig / err.max(1e-20)).log10()
                    }
                    None => f64::INFINITY,
                };
                if device == Device::Cpu {
                    reference = Some(vocals);
                }
                Ok(Row {
                    model: "stems",
                    device: device.name(),
                    ok: true,
                    load_s,
                    run_s,
                    fidelity: if fidelity.is_finite() { fidelity } else { 999.0 },
                    note: format!("{stem_secs:.1} s of audio, RTF {:.3}, SNR dB vs CPU", run_s / stem_secs),
                })
            })();
            push(&mut rows, "stems", device, row);
        }
    }

    if let Some(path) = json {
        std::fs::write(&path, serde_json::to_vec_pretty(&rows)?).with_context(|| format!("write {path}"))?;
    }
    Ok(())
}

fn push(rows: &mut Vec<Row>, model: &'static str, device: Device, row: Result<Row>) {
    if let Err(e) = &row {
        eprintln!("[{model} on {}] {e:#}", device.name());
    }
    let row = row.unwrap_or_else(|e| Row {
        model,
        device: device.name(),
        ok: false,
        load_s: 0.0,
        run_s: 0.0,
        fidelity: 0.0,
        note: format!("{e:#}").chars().take(160).collect(),
    });
    println!("{:<8} {:<9} {:>8.2} {:>9.2} {:>10.2}  {}", row.model, row.device, row.load_s, row.run_s, row.fidelity, row.note);
    rows.push(row);
}
