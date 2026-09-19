//! Device planning: which processor runs which model on this machine.
//!
//! Nothing is assumed. Each model is timed on every available device with a synthetic
//! piece of music and compared with the CPU result; a device is chosen only if it is
//! clearly faster AND numerically faithful. On the development laptop this rejects the
//! Intel Arc iGPU for chords (14 % of labels change) while accepting the RTX 3050 for beat
//! tracking (3x faster, identical beats). See docs/ACCELERATION.md.

use std::ops::ControlFlow;
use std::path::Path;
use std::time::Instant;

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::beats::BeatTracker;
use crate::btc::ChordModel;
use crate::ortutil::Device;
use crate::separate::htdemucs::{HtDemucs, SEGMENT};
use crate::separate::Separator;

/// A candidate must beat the CPU by this factor to be worth the extra moving part.
const MIN_SPEEDUP: f64 = 1.25;
const MIN_LABEL_AGREEMENT: f64 = 0.995;
const MIN_BEAT_AGREEMENT: f64 = 0.99;
const MIN_STEM_SNR_DB: f64 = 60.0;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Plan {
    pub chords: Device,
    pub beats: Device,
    pub stems: Device,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Trial {
    pub model: String,
    pub device: Device,
    pub available: bool,
    pub load_s: f64,
    pub run_s: f64,
    /// Label agreement (chords), share of CPU beats matched (beats), SNR in dB (stems).
    pub fidelity: f64,
    pub accepted: bool,
    pub note: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Calibration {
    pub plan: Plan,
    pub trials: Vec<Trial>,
}

/// Devices worth trying besides the CPU, fastest expectation first.
pub const CANDIDATES: [Device; 4] = [Device::Cuda, Device::GpuHigh, Device::GpuLow, Device::Npu];

/// 40 s of band-like audio at `rate`: a I-vi-IV-V pad with a kick / snare / hat pattern.
pub fn test_signal(rate: u32, seconds: f32) -> Vec<f32> {
    let n = (rate as f32 * seconds) as usize;
    let sr = rate as f32;
    let beat = 0.5f32; // 120 BPM
    let prog: [[i32; 3]; 4] = [[0, 4, 7], [9, 12, 16], [5, 9, 12], [7, 11, 14]];
    let mut seed = 0x9E37_79B9u32;
    let mut noise = move || {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        seed as f32 / u32::MAX as f32 * 2.0 - 1.0
    };
    (0..n)
        .map(|i| {
            let t = i as f32 / sr;
            let bar = (t / (4.0 * beat)) as usize % 4;
            let in_beat = t % beat;
            let beat_no = (t / beat) as usize % 4;
            let mut v = 0.0;
            for semis in prog[bar] {
                let f = 261.63 * 2f32.powf(semis as f32 / 12.0);
                for h in 1..4 {
                    v += 0.06 / h as f32 * (2.0 * std::f32::consts::PI * f * h as f32 * t).sin();
                }
            }
            v += 0.12 * (2.0 * std::f32::consts::PI * 65.41 * 2f32.powf(prog[bar][0] as f32 / 12.0) * t).sin();
            let env = (-in_beat * 22.0).exp();
            if beat_no % 2 == 0 {
                v += 0.6 * env * (2.0 * std::f32::consts::PI * (50.0 + 90.0 * (-in_beat * 35.0).exp()) * in_beat).sin();
            } else {
                v += 0.3 * env * noise();
            }
            v += 0.04 * (-(t % (beat / 2.0)) * 70.0).exp() * noise();
            v * 0.6
        })
        .collect()
}

fn trial(model: &str, device: Device) -> Trial {
    Trial { model: model.into(), device, available: false, load_s: 0.0, run_s: 0.0, fidelity: 0.0, accepted: false, note: String::new() }
}

fn timed<T>(f: impl FnOnce() -> Result<T>) -> Result<(T, f64)> {
    let t = Instant::now();
    let v = f()?;
    Ok((v, t.elapsed().as_secs_f64()))
}

/// Pick the fastest accepted trial, or the CPU.
fn choose(trials: &mut [Trial], cpu_run: f64, min_fidelity: f64) -> Device {
    let mut best = (Device::Cpu, cpu_run / MIN_SPEEDUP);
    for t in trials.iter_mut().filter(|t| t.available && t.device != Device::Cpu) {
        if t.fidelity < min_fidelity {
            t.note = format!("ditolak: hasil menyimpang dari CPU ({:.2})", t.fidelity);
        } else if t.run_s > cpu_run / MIN_SPEEDUP {
            t.note = "ditolak: tidak cukup lebih cepat dari CPU".into();
        } else if t.run_s < best.1 {
            best = (t.device, t.run_s);
        }
    }
    for t in trials.iter_mut() {
        t.accepted = t.device == best.0;
    }
    best.0
}

/// Time chords and beats on every device. `candidates` lets callers skip devices.
pub fn calibrate_analysis(btc: &Path, mel: &Path, beat: &Path, threads: usize, candidates: &[Device]) -> Calibration {
    let y = test_signal(22_050, 40.0);
    let mut out = Calibration::default();
    let devices: Vec<Device> = std::iter::once(Device::Cpu).chain(candidates.iter().copied()).collect();

    let mut chord_trials = Vec::new();
    let (mut cpu_labels, mut cpu_run): (Vec<u16>, f64) = (Vec::new(), f64::MAX);
    for &device in &devices {
        let mut t = trial("chords", device);
        let result = (|| -> Result<()> {
            let (mut model, load_s) = timed(|| ChordModel::load_on(btc, device, threads))?;
            model.analyze(&y[..22_050 * 12])?; // warm-up: kernel / shader compilation
            let (post, run_s) = timed(|| model.analyze(&y))?;
            let labels: Vec<u16> = (0..post.frames).map(|f| post.argmax(f)).collect();
            if device == Device::Cpu {
                cpu_labels = labels.clone();
                cpu_run = run_s;
            }
            let same = cpu_labels.iter().zip(&labels).filter(|(a, b)| a == b).count();
            t = Trial { available: true, load_s, run_s, fidelity: same as f64 / cpu_labels.len().max(1) as f64, ..t.clone() };
            Ok(())
        })();
        if let Err(e) = result {
            t.note = short(&e);
        }
        chord_trials.push(t);
    }
    out.plan.chords = choose(&mut chord_trials, cpu_run, MIN_LABEL_AGREEMENT);
    out.trials.extend(chord_trials);

    let mut beat_trials = Vec::new();
    let (mut cpu_beats, mut cpu_run): (Vec<f64>, f64) = (Vec::new(), f64::MAX);
    for &device in &devices {
        let mut t = trial("beats", device);
        let result = (|| -> Result<()> {
            let (mut model, load_s) = timed(|| BeatTracker::load_on(mel, beat, device, threads))?;
            model.analyze(&y[..22_050 * 31])?;
            let (grid, run_s) = timed(|| model.analyze(&y))?;
            if device == Device::Cpu {
                cpu_beats = grid.beats.clone();
                cpu_run = run_s;
            }
            let hit = cpu_beats.iter().filter(|b| grid.beats.iter().any(|g| (g - **b).abs() <= 0.02)).count();
            let extra = grid.beats.len().saturating_sub(cpu_beats.len());
            let fidelity = hit.saturating_sub(extra) as f64 / cpu_beats.len().max(1) as f64;
            t = Trial { available: true, load_s, run_s, fidelity, ..t.clone() };
            Ok(())
        })();
        if let Err(e) = result {
            t.note = short(&e);
        }
        beat_trials.push(t);
    }
    out.plan.beats = choose(&mut beat_trials, cpu_run, MIN_BEAT_AGREEMENT);
    out.trials.extend(beat_trials);
    out
}

/// Time the separator on one 7.8 s segment per device.
pub fn calibrate_stems(onnx: &Path, threads: usize, candidates: &[Device]) -> (Device, Vec<Trial>) {
    let mono = test_signal(44_100, SEGMENT as f32 / 44_100.0 + 0.01);
    let left: Vec<f32> = mono[..SEGMENT].to_vec();
    let right: Vec<f32> = left.iter().enumerate().map(|(i, v)| v * (0.8 + 0.2 * ((i / 4410) % 2) as f32)).collect();
    let mix = [left, right];

    let mut trials = Vec::new();
    let (mut cpu_out, mut cpu_run): (Vec<f32>, f64) = (Vec::new(), f64::MAX);
    for device in std::iter::once(Device::Cpu).chain(candidates.iter().copied()) {
        let mut t = trial("stems", device);
        let result = (|| -> Result<()> {
            let (mut model, load_s) = timed(|| HtDemucs::load_on(onnx, device, threads))?;
            let run = |model: &mut HtDemucs| -> Result<Vec<f32>> {
                let mut got = Vec::new();
                model.separate(&mix, &mut |block| {
                    for stem in block.stems {
                        got.extend_from_slice(&stem[0]);
                    }
                    Ok(ControlFlow::Continue(()))
                })?;
                Ok(got)
            };
            if device != Device::Cpu {
                run(&mut model)?; // warm-up
            }
            let (got, run_s) = timed(|| run(&mut model))?;
            if device == Device::Cpu {
                cpu_out = got.clone();
                cpu_run = run_s;
            }
            let sig: f64 = cpu_out.iter().map(|v| (*v as f64).powi(2)).sum();
            let err: f64 = cpu_out.iter().zip(&got).map(|(a, b)| ((a - b) as f64).powi(2)).sum();
            let snr = if err <= 0.0 { 200.0 } else { 10.0 * (sig / err).log10() };
            t = Trial { available: true, load_s, run_s, fidelity: snr, ..t.clone() };
            Ok(())
        })();
        if let Err(e) = result {
            t.note = short(&e);
        }
        trials.push(t);
    }
    let device = choose(&mut trials, cpu_run, MIN_STEM_SNR_DB);
    (device, trials)
}

fn short(e: &anyhow::Error) -> String {
    let text = format!("{e:#}");
    // ONNX Runtime errors carry build paths; keep the human part.
    let tail = text.rsplit("onnxruntime::").next().unwrap_or(&text);
    tail.chars().take(180).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(device: Device, run_s: f64, fidelity: f64) -> Trial {
        Trial { available: true, run_s, fidelity, ..trial("x", device) }
    }

    #[test]
    fn unfaithful_or_slow_devices_are_rejected() {
        let mut trials = vec![t(Device::Cpu, 6.0, 1.0), t(Device::GpuHigh, 2.0, 1.0), t(Device::GpuLow, 1.0, 0.86), t(Device::Npu, 5.5, 1.0)];
        assert_eq!(choose(&mut trials, 6.0, 0.995), Device::GpuHigh);
        assert!(trials[1].accepted && !trials[2].accepted);
        assert!(trials[2].note.contains("menyimpang") && trials[3].note.contains("tidak cukup"));
    }

    #[test]
    fn cpu_wins_when_nothing_is_clearly_better() {
        let mut trials = vec![t(Device::Cpu, 1.0, 1.0), t(Device::GpuHigh, 0.95, 1.0)];
        assert_eq!(choose(&mut trials, 1.0, 0.995), Device::Cpu);
        assert!(trials[0].accepted);
    }

    #[test]
    fn test_signal_is_loud_enough_and_bounded() {
        let y = test_signal(22_050, 4.0);
        let peak = y.iter().fold(0f32, |m, v| m.max(v.abs()));
        assert!(peak > 0.2 && peak < 1.0, "peak {peak}");
    }
}
