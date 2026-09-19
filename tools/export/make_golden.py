"""Golden reference data for the Rust parity tests (committed, synthetic audio only).

  synth_22k.f32        mono 22.05 kHz PCM: a I-vi-IV-V progression of harmonic tones + noise
  synth_logcqt_lib.f32 log|librosa.cqt| of the first 10 s instance        [108, 144]
  synth_logcqt_dir.f32 same instance, single-rate direct CQT (proto_cqt)   [108, 144]
  synth_logits.f32     BTC logits (PyTorch) for synth_logcqt_lib           [108, 170]
"""
import numpy as np
import librosa
import torch

from common import BPO, GOLDEN, HOP, N_BINS, SR, TIMESTEP, save_f32, write_manifest
from export_btc import load
from common import BTC_DIR
from proto_cqt import direct_cqt


def synth(seconds=20.0, seed=7):
    rng = np.random.default_rng(seed)
    prog = [(0, [0, 4, 7]), (9, [0, 3, 7]), (5, [0, 4, 7]), (7, [0, 4, 7, 10])]  # C Am F G7
    n = int(seconds * SR)
    t = np.arange(n) / SR
    y = np.zeros(n)
    bar = 2.5
    for b in range(int(np.ceil(seconds / bar))):
        root, ivs = prog[b % len(prog)]
        lo, hi = int(b * bar * SR), min(int((b + 1) * bar * SR), n)
        seg = t[lo:hi] - t[lo]
        env = np.minimum(1.0, seg / 0.02) * np.exp(-seg * 0.6)
        notes = [36 + root] + [60 + ((root + iv) % 12) for iv in ivs]
        for midi in notes:
            f0 = 440.0 * 2 ** ((midi - 69) / 12)
            for h in range(1, 7):
                if f0 * h < SR / 2:
                    y[lo:hi] += env * np.sin(2 * np.pi * f0 * h * seg + rng.uniform(0, 6.28)) / h**1.5
    y += rng.standard_normal(n) * 0.003
    return (0.5 * y / np.abs(y).max()).astype(np.float32), [(r, q) for r, q in prog]


def main():
    manifest = {"sr": SR, "hop": HOP, "n_bins": N_BINS, "bpo": BPO, "timestep": TIMESTEP,
                "librosa": librosa.__version__, "torch": torch.__version__,
                "progression_bar_secs": 2.5, "progression_classes": [1, 9 * 14 + 0, 5 * 14 + 1, 7 * 14 + 9]}
    y, _ = synth()
    save_f32(GOLDEN / "synth_22k.f32", y, manifest)
    inst = y[: SR * 10]
    lib = np.log(np.abs(librosa.cqt(inst, sr=SR, n_bins=N_BINS, bins_per_octave=BPO, hop_length=HOP, pad_mode="reflect")) + 1e-6).T
    ypad_reflect = inst  # direct_cqt pads with zeros; emulate reflect by padding here
    n_fft = 32768
    padded = np.pad(inst, n_fft // 2, mode="reflect")
    dire = direct_cqt(padded)[:, n_fft // 2 // HOP : n_fft // 2 // HOP + TIMESTEP]
    dire = np.log(np.abs(dire) + 1e-6).T
    assert lib.shape == (TIMESTEP, N_BINS) == dire.shape, (lib.shape, dire.shape)
    save_f32(GOLDEN / "synth_logcqt_lib.f32", lib, manifest)
    save_f32(GOLDEN / "synth_logcqt_dir.f32", dire, manifest)
    model = load(str(BTC_DIR / "test" / "btc_model_large_voca.pt"))
    with torch.no_grad():
        logits = model(torch.from_numpy(lib.astype(np.float32))[None]).numpy()[0]
    save_f32(GOLDEN / "synth_logits.f32", logits, manifest)
    lab = logits.argmax(-1)
    print("lib vs direct log-CQT: mean diff %.5f std %.5f" % ((dire - lib).mean(), (dire - lib).std()))
    print("labels per 2.5 s bar:", [int(np.bincount(lab[int(b * 2.5 * SR / HOP) + 3 : int((b + 1) * 2.5 * SR / HOP) - 3]).argmax()) for b in range(4)])
    write_manifest(GOLDEN / "manifest.json", manifest)


if __name__ == "__main__":
    main()
