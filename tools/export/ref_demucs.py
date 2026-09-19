"""Parity check for chordku-core's htdemucs DSP against the reference PyTorch Demucs.

  python ref_demucs.py mix_44k_stereo.wav rust_stems_dir

Runs demucs.apply.apply_model(htdemucs, shifts=0, overlap=0.25) exactly like demucs.separate
and prints the SNR of each Rust stem against the reference stem.
"""
import sys
from pathlib import Path

import numpy as np
import soundfile as sf
import torch
from demucs.apply import apply_model
from demucs.pretrained import get_model

mix, sr = sf.read(sys.argv[1], dtype="float32", always_2d=True)
assert sr == 44100 and mix.shape[1] == 2
wav = torch.from_numpy(mix.T.copy())
model = get_model("htdemucs")
model.eval()
ref = wav.mean(0)
wav_n = (wav - ref.mean()) / ref.std()
with torch.no_grad():
    sources = apply_model(model, wav_n[None], shifts=0, split=True, overlap=0.25, progress=False, device="cpu")[0]
sources = sources * ref.std() + ref.mean()
print("sources:", model.sources)
worst = 1e9
for name, est in zip(model.sources, sources):
    rust, _ = sf.read(str(Path(sys.argv[2]) / f"{name}.wav"), dtype="float32", always_2d=True)
    r = est.numpy().T
    n = min(len(r), len(rust))
    err = rust[:n] - r[:n]
    snr = 10 * np.log10((r[:n] ** 2).sum() / max((err ** 2).sum(), 1e-20))
    worst = min(worst, snr)
    print(f"{name:7s} SNR {snr:6.1f} dB   max|err| {np.abs(err).max():.2e}   ref rms {np.sqrt((r**2).mean()):.4f}")
recon = sources.sum(0).numpy().T
print(f"sum of stems vs mix: SNR {10*np.log10((mix**2).sum() / ((recon - mix)**2).sum()):.1f} dB")
print("PASS" if worst >= 40 else "FAIL", f"(worst {worst:.1f} dB)")
