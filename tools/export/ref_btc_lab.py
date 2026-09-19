"""Reference labels exactly as BTC's test.py produces them (librosa per 10 s chunk, hard seams)."""
import sys
import librosa, numpy as np, onnxruntime as ort
from common import BPO, HOP, MODELS_OUT, N_BINS, SR, TIMESTEP

ROOTS = ['C', 'C#', 'D', 'D#', 'E', 'F', 'F#', 'G', 'G#', 'A', 'A#', 'B']
QUAL = ['min', 'maj', 'dim', 'aug', 'min6', 'maj6', 'min7', 'minmaj7', 'maj7', '7', 'dim7', 'hdim7', 'sus2', 'sus4']
name = lambda i: 'N' if i == 169 else 'X' if i == 168 else ROOTS[i // 14] + ('' if i % 14 == 1 else ':' + QUAL[i % 14])

y, _ = librosa.load(sys.argv[1], sr=SR, mono=True)
sess = ort.InferenceSession(str(MODELS_OUT / "btc.onnx"), providers=["CPUExecutionProvider"])
chunk = SR * 10
feat = np.concatenate([librosa.cqt(y[i:i + chunk], sr=SR, n_bins=N_BINS, bins_per_octave=BPO, hop_length=HOP) for i in range(0, len(y), chunk)], axis=1)
feat = np.log(np.abs(feat) + 1e-6).T.astype(np.float32)
n = len(feat); pad = (-n) % TIMESTEP
lab = sess.run(None, {"logcqt": np.pad(feat, ((0, pad), (0, 0)), constant_values=-2.227988).reshape(-1, TIMESTEP, N_BINS)})[0].argmax(-1).reshape(-1)[:n]
unit = 10.0 / TIMESTEP
with open(sys.argv[2], "w") as f:
    s = 0
    for t in range(1, n + 1):
        if t == n or lab[t] != lab[s]:
            f.write(f"{s * unit:.3f}\t{t * unit:.3f}\t{name(lab[s])}\n"); s = t
