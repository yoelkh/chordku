# Hardware acceleration

ChordKu measures every model on every processor of the machine and uses a device only when it
is clearly faster **and** gives the same result as the CPU. Nothing is assumed from a spec sheet.

```
chordku-cli bench song.mp3              # full table: speed and fidelity per model and device
chordku-cli bench --calibrate           # exactly what the server does on first start
```

The server calibrates chords, beats and (when its model is installed) the separator on its first
start, about 30 s, once per build / runtime / model combination, and again right after a model
is downloaded. The plan is stored in the database. While stems are being separated the progress
banner names the device ("... di GPU NVIDIA (CUDA)"). **Pengaturan > Akselerasi perangkat keras** shows the plan, the measurements, and a
button to measure again. If an accelerated session ever fails to load (driver update, GPU busy),
the CPU is used and the app keeps working.

## What this laptop has

Core Ultra 5 125H (14 cores), NVIDIA RTX 3050 Laptop 6 GB, Intel Arc iGPU, Intel AI Boost NPU.

| Path | Reaches | Needs |
|---|---|---|
| CPU | everything | nothing |
| DirectML | RTX 3050, Arc | nothing: ships inside the ONNX Runtime build we link |
| CUDA + cuDNN | RTX 3050 | NVIDIA's runtime DLLs (about 1.5 GB), downloaded from inside the app |
| OpenVINO | NPU, Arc, CPU | a second inference runtime; evaluated, not adopted (see below) |

## Measurements (4 min 17 s song)

| Model | CPU | RTX 3050, CUDA | RTX 3050, DirectML | Arc, DirectML | NPU |
|---|---|---|---|---|---|
| Beats (Beat This! small) | 3.95 s | **1.09 s**, identical beats | 1.91 s, identical | 4.03 s, identical | does not compile (OpenVINO, driver 2696) |
| Chords (CQT + BTC) | **0.85 s** | 0.64 s, identical labels | 0.77 s | 2.82 s, **14 % of labels change** | BTC alone: 0.29 s vs 0.35 s on CPU, 12 s compile |
| Stems (htdemucs), per 7.8 s segment | 3.96 s | float32 0.75 s, 74 dB SNR vs CPU; **float16 0.49 s**, 55 dB | fails to initialise | fails to initialise | 1.18 s, **wrong output**, 240 s compile |

Resulting plan on this machine: **beats on CUDA, chords on the CPU, stems on CUDA in half precision.**

| End to end | before | now |
|---|---|---|
| upload until the chart is visible | about 7 s | **4.1 s** |
| upload until audio is ready too | 10.5 s | **7.8 s** |
| stem separation of the same song | about 100 s | **12 s** |

## Why each decision

**Chords stay on the CPU.** The network is 3 M parameters; nearly all the time is the CQT, which
is CPU code. It is now computed for all 10 s instances in parallel (0.5 s became 0.15 s). CUDA
is 25 % faster end to end, below the 1.25x threshold that justifies another moving part.

**The Arc iGPU is rejected for chords.** It is slower than the CPU for this model and its
results differ: 14 % of frame labels change on a real song. Integrated-GPU numerical divergence
under DirectML is a known ONNX Runtime issue; the calibration catches it instead of trusting it.
For beats it is faithful but slower than the NVIDIA GPU.

**htdemucs needs CUDA.** DirectML refuses to initialise this graph on both GPUs ("the parameter
is incorrect", at every graph optimisation level). CUDA runs it 4.3x faster per segment. Two
things were needed to get that speed inside the server:

- *GPU memory discipline.* ONNX Runtime's CUDA arena grows in powers of two and never shrinks.
  With the beat model resident, htdemucs spilled into shared system memory and ran **10x slower
  than the CPU** (10.0 s per segment). Sessions now grow by exactly what they request, and the
  small analysis models are capped at 1 GB. GPU memory after a job: 0.8 GB.
- *Keeping the GPU fed.* At 0.5 s per segment the serial CPU work around the network (eight
  inverse STFTs, six Opus encoders) took as long as the network. The inverse STFTs now run one
  thread per stem and the encoders on their own thread behind a two-block channel: 47.6 s
  became 30.1 s. The CPU-only path gained too (RTF 0.38 to 0.30). Parity with PyTorch Demucs
  is unchanged (73.5-100 dB).

**Half precision for the separator.** In float32 htdemucs peaks at 5.9 GB of video memory, the
whole of a 6 GB laptop GPU. Alone in a process that just fits (17 s for a 4 min 19 s song);
inside the server, next to the beat model and the desktop, it spills into shared system memory
and the same job takes 39 s. A float16 copy of the graph peaks at 3.3 GB (4.0 GB for the whole
server) and the network itself is 1.5x faster: **12 s** for the job.

- The copy is made on the user's machine by `separate/half.rs` the first time it is needed
  (`htdemucs_fwd.fp16.onnx`, 84 MB, about a second): the pretrained weights may not be
  redistributed, so it cannot ship. Inputs and outputs stay float32. The converter refuses any
  graph containing an operator it was not verified on, which leaves float32 in use.
- It is a calibration candidate like any other device (`cuda_fp16`). Its fidelity gate is 40 dB
  instead of 60 dB, because half precision cannot reach 60. Measured on four full songs it lands
  at 50-57 dB on the vocals and 44-50 dB on the other stems, with every sample finite.
- What the difference sounds like was checked against something already accepted: the stems are
  stored as 96 kbit/s Opus. Over a song the float16 error is a median of 17 dB *below* the Opus
  coding error of the same stem; it exceeds it in 5 % of two-second windows (quiet bass under a
  loud mix), by at most 7 dB, at an absolute level of -58 dBFS.
- If a segment ever overflows float16 (non-finite output), the separator reloads the float32
  graph and repeats that segment. `CHORDKU_NO_HALF=1` keeps everything in float32.
- Not adopted after measuring: keeping the normalisation layers in float32 (+1 dB fidelity for
  a third of the speed gain), cuDNN heuristic search and workspace limits (no effect on time or
  memory), lowering the process priority only on the CPU path (no effect).

**The NPU is not used, and that is a measured result, not an omission.**

- DirectML does not expose it on this system ("no devices match the filter"), so it was tested
  through OpenVINO 2026.4 (`tools/export/bench_openvino.py`).
- Beat This!: compilation fails (`ZE_RESULT_ERROR_INVALID_ARGUMENT`).
- BTC: works and is faithful (99.93 % identical labels) but saves 60 ms per song after a 12 s
  compile, and would require shipping OpenVINO as a second runtime.
- htdemucs: compiles for four minutes, is no faster than the CPU, and the output is wrong
  (maximum error 3.25 on a range of 4.6): the NPU's reduced precision does not survive this
  network.
- The Meteor Lake NPU is an 11 TOPS part built for small, always-on, low-power models (camera
  effects, noise suppression). These three networks are either too small to benefit or too
  large and precision-sensitive to run correctly.

**OpenVINO as a future option.** On the CPU, OpenVINO ran an htdemucs segment in 1.1 s against
ONNX Runtime's 3-4 s, and the Arc iGPU in 0.43 s (with reduced precision). For a machine
without an NVIDIA GPU that would be worth a second runtime behind the `Separator` trait. Here
CUDA already wins, so it was not added.

## Build notes

- The `cuda` cargo feature is on by default. It switches ONNX Runtime from the fully static
  build to the prebuilt `cuda13 + directml` variant: the core is still linked into the exe, and
  `onnxruntime_providers_cuda.dll`, `onnxruntime_providers_shared.dll` and `DirectML.dll` sit
  next to it. Build without CUDA: `cargo build --release --no-default-features`.
- NVIDIA's DLLs are looked up in `CHORDKU_CUDA`, else `<data>/cuda`, and put on the process DLL
  search path before the first session is created. They come from NVIDIA's official PyPI
  wheels, pinned by SHA-256 (`crates/chordku-server/src/cuda.rs`).
- `CHORDKU_NO_ACCEL=1` skips calibration and keeps everything on the CPU. `CHORDKU_NO_HALF=1`
  keeps the separator in float32.
