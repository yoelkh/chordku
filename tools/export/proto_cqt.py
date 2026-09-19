"""Prototype: single-rate FFT CQT (the algorithm chordku-core implements in Rust)
compared with librosa.cqt (recursive down-sampling), in the log domain BTC consumes.

Run:  python proto_cqt.py [audio] [seconds]
"""
import sys

import librosa
import numpy as np

from common import BPO, BTC_DIR, HOP, N_BINS, SR


def direct_cqt(y: np.ndarray, sr=SR, hop=HOP, n_bins=N_BINS, bpo=BPO) -> np.ndarray:
    fmin = librosa.note_to_hz("C1")
    freqs = fmin * 2.0 ** (np.arange(n_bins) / bpo)
    basis, lengths = librosa.filters.wavelet(freqs=freqs, sr=sr, filter_scale=1, norm=1, pad_fft=True, window="hann")
    n_fft = basis.shape[1]
    fft_basis = np.fft.fft(basis, n=n_fft, axis=1)[:, : n_fft // 2 + 1]
    ypad = np.pad(y, n_fft // 2, mode="constant")
    n_frames = 1 + (len(ypad) - n_fft) // hop
    out = np.empty((n_bins, n_frames), dtype=np.complex64)
    for t in range(n_frames):
        spec = np.fft.rfft(ypad[t * hop : t * hop + n_fft])
        out[:, t] = fft_basis @ spec
    # librosa: basis *= lengths / n_fft, then (scale=True) C /= sqrt(lengths)  =>  C * sqrt(L) / n_fft
    return out * (np.sqrt(lengths) / n_fft)[:, None]


def main():
    path = sys.argv[1] if len(sys.argv) > 1 else str(BTC_DIR / "test" / "example.mp3")
    secs = float(sys.argv[2]) if len(sys.argv) > 2 else 30.0
    y, _ = librosa.load(path, sr=SR, mono=True, duration=secs)
    ref = np.log(np.abs(librosa.cqt(y, sr=SR, n_bins=N_BINS, bins_per_octave=BPO, hop_length=HOP)) + 1e-6)
    got = np.log(np.abs(direct_cqt(y)) + 1e-6)
    n = min(ref.shape[1], got.shape[1])
    ref, got = ref[:, :n], got[:, :n]
    d = got - ref
    print(f"frames={n} ref mean={ref.mean():.3f} std={ref.std():.3f}")
    print(f"log diff: mean={d.mean():+.4f} std={d.std():.4f} max|d|={np.abs(d).max():.3f}")
    for o in range(N_BINS // BPO):
        do = d[o * BPO : (o + 1) * BPO]
        print(f"  octave {o}: mean={do.mean():+.4f} std={do.std():.4f}")
    inner = d[:, 6:-6]
    print(f"inner frames: mean={inner.mean():+.4f} std={inner.std():.4f} max|d|={np.abs(inner).max():.3f}")
    print(f"corr={np.corrcoef(ref.ravel(), got.ravel())[0, 1]:.6f}")


if __name__ == "__main__":
    main()
