//! Shared ONNX Runtime session construction and device selection.
//!
//! On Windows the accelerators are reached through DirectML, which ships inside the
//! ONNX Runtime build we link: no CUDA toolkit, cuDNN or OpenVINO install is needed, and
//! the same code path covers the discrete GPU, the integrated GPU and the NPU.
//! Which device actually wins for which model is measured, not assumed
//! (`chordku-cli bench`, see docs/ACCELERATION.md).

use std::path::Path;

use anyhow::{anyhow, Context, Result};
use ort::ep;
use ort::session::builder::GraphOptimizationLevel;
use ort::session::Session;
use serde::{Deserialize, Serialize};

/// Where a model runs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Device {
    #[default]
    Cpu,
    /// The fastest GPU DirectML can find (the discrete NVIDIA GPU on a hybrid laptop).
    GpuHigh,
    /// The power-saving GPU (the integrated Intel Arc on a hybrid laptop).
    GpuLow,
    /// Neural processing unit (Intel AI Boost).
    Npu,
    /// NVIDIA GPU through CUDA + cuDNN (needs the `cuda` build feature and NVIDIA's runtime
    /// libraries on the DLL search path). Covers graphs DirectML cannot run, like htdemucs.
    Cuda,
}

impl Device {
    pub const ALL: [Device; 5] = [Device::Cpu, Device::Cuda, Device::GpuHigh, Device::GpuLow, Device::Npu];

    pub fn name(self) -> &'static str {
        match self {
            Device::Cpu => "cpu",
            Device::Cuda => "cuda",
            Device::GpuHigh => "gpu_high",
            Device::GpuLow => "gpu_low",
            Device::Npu => "npu",
        }
    }

    pub fn parse(text: &str) -> Option<Device> {
        Device::ALL.into_iter().find(|d| d.name() == text)
    }
}

/// Build a CPU session. `threads = 0` lets ONNX Runtime pick; spinning is disabled so
/// idle intra-op workers sleep instead of burning a core between calls.
pub fn cpu_session(model: &Path, threads: usize) -> Result<Session> {
    session(model, Device::Cpu, threads, false)
}

/// Like [`cpu_session`] but without the growing arena allocator and memory-pattern
/// planning. Large models that run a few long calls (stem separation) otherwise keep
/// gigabytes of activation memory reserved for the life of the session.
pub fn cpu_session_low_memory(model: &Path, threads: usize) -> Result<Session> {
    session(model, Device::Cpu, threads, true)
}

/// Build a session on `device`. Fails (instead of silently falling back to the CPU) when
/// the device or one of the model's operators is not available there, so callers can make
/// the fallback explicit and report it.
pub fn session(model: &Path, device: Device, threads: usize, low_memory: bool) -> Result<Session> {
    // Experiment knob (not a user setting): CHORDKU_ORT_OPT=0..3 overrides graph optimisation.
    let level = match std::env::var("CHORDKU_ORT_OPT").ok().as_deref() {
        Some("0") => GraphOptimizationLevel::Disable,
        Some("1") => GraphOptimizationLevel::Level1,
        Some("2") => GraphOptimizationLevel::Level2,
        _ => GraphOptimizationLevel::Level3,
    };
    let mut builder = Session::builder()
        .map_err(|e| anyhow!("{e}"))?
        .with_optimization_level(level)
        .map_err(|e| anyhow!("{e}"))?
        .with_intra_op_spinning(false)
        .map_err(|e| anyhow!("{e}"))?;
    match device {
        Device::Cpu => {
            if low_memory {
                builder = builder
                    .with_memory_pattern(false)
                    .map_err(|e| anyhow!("{e}"))?
                    .with_execution_providers([ep::CPU::default().with_arena_allocator(false).build()])
                    .map_err(|e| anyhow!("{e}"))?;
            }
        }
        Device::Cuda => {
            #[cfg(feature = "cuda")]
            {
                // Several sessions share one 6 GB laptop GPU. The default arena grows in powers
                // of two and never shrinks, so a small model can sit on gigabytes and push the
                // next one into (10x slower) shared system memory. Grow by exactly what is
                // requested, and cap the small analysis models.
                let mut cuda = ep::CUDA::default()
                    .with_device_id(0)
                    .with_arena_extend_strategy(ort::ep::ArenaExtendStrategy::SameAsRequested);
                if !low_memory {
                    cuda = cuda.with_memory_limit(1024 * 1024 * 1024);
                }
                builder = builder
                    .with_execution_providers([cuda.build().error_on_failure()])
                    .map_err(|e| anyhow!("cuda tidak tersedia: {e}"))?;
            }
            #[cfg(not(feature = "cuda"))]
            anyhow::bail!("this build has no CUDA support (cargo feature `cuda`)");
        }
        accelerated => {
            #[cfg(windows)]
            {
                use ort::ep::directml::{DeviceFilter, PerformancePreference};
                let dml = match accelerated {
                    Device::GpuHigh => ep::DirectML::default()
                        .with_device_filter(DeviceFilter::Gpu)
                        .with_performance_preference(PerformancePreference::HighPerformance),
                    Device::GpuLow => ep::DirectML::default()
                        .with_device_filter(DeviceFilter::Gpu)
                        .with_performance_preference(PerformancePreference::MinimumPower),
                    _ => ep::DirectML::default().with_device_filter(DeviceFilter::Npu),
                };
                // DirectML requires sequential execution without memory-pattern planning.
                builder = builder
                    .with_memory_pattern(false)
                    .map_err(|e| anyhow!("{e}"))?
                    .with_parallel_execution(false)
                    .map_err(|e| anyhow!("{e}"))?
                    .with_execution_providers([dml.build().error_on_failure()])
                    .map_err(|e| anyhow!("{} tidak tersedia: {e}", accelerated.name()))?;
            }
            #[cfg(not(windows))]
            {
                let _ = accelerated;
                anyhow::bail!("hardware acceleration is implemented through DirectML (Windows only)");
            }
        }
    }
    if threads > 0 {
        builder = builder.with_intra_threads(threads).map_err(|e| anyhow!("{e}"))?;
    }
    builder.commit_from_file(model).with_context(|| format!("load {} on {}", model.display(), device.name()))
}
