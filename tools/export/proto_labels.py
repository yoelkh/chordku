"""Label agreement: reference BTC pipeline (librosa.cqt per 10 s chunk) vs chordku's
continuous direct CQT with 50%-overlapped windows keeping the centre frames."""
import sys, time
import librosa, numpy as np, onnxruntime as ort
from common import BPO, BTC_DIR, HOP, MODELS_OUT, N_BINS, SR, TIMESTEP
from proto_cqt import direct_cqt

path = sys.argv[1] if len(sys.argv) > 1 else str(BTC_DIR / "test" / "example.mp3")
y, _ = librosa.load(path, sr=SR, mono=True)
sess = ort.InferenceSession(str(MODELS_OUT / "btc.onnx"), providers=["CPUExecutionProvider"])

# --- reference: exactly what BTC test.py does
chunk = SR * 10
feats = [librosa.cqt(y[i:i + chunk], sr=SR, n_bins=N_BINS, bins_per_octave=BPO, hop_length=HOP) for i in range(0, len(y), chunk)]
ref_feat = np.log(np.abs(np.concatenate(feats, axis=1)) + 1e-6).T.astype(np.float32)
pad = (-len(ref_feat)) % TIMESTEP
x = np.pad(ref_feat, ((0, pad), (0, 0))).reshape(-1, TIMESTEP, N_BINS)
ref_lab = sess.run(None, {"logcqt": x})[0].argmax(-1).reshape(-1)[: len(ref_feat)]
ref_t = np.arange(len(ref_lab)) * (10.0 / TIMESTEP)

# --- chordku: continuous CQT, hop 54 windows, keep centre 54 frames
t0 = time.time()
feat = np.log(np.abs(direct_cqt(y)) + 1e-6).T.astype(np.float32)
t_cqt = time.time() - t0
n = len(feat); half = TIMESTEP // 2; q = TIMESTEP // 4
starts = list(range(0, max(n - TIMESTEP, 0) + 1, half)) or [0]
if starts[-1] + TIMESTEP < n: starts.append(n - TIMESTEP)
logits = np.zeros((n, 170), np.float32); filled = np.zeros(n, bool)
fp = np.pad(feat, ((0, max(0, TIMESTEP - n)), (0, 0)))
batch = np.stack([fp[s:s + TIMESTEP] for s in starts])
out = sess.run(None, {"logcqt": batch})[0]
for w, s in enumerate(starts):
    lo = 0 if w == 0 else q
    hi = TIMESTEP if w == len(starts) - 1 else TIMESTEP - q
    for i in range(lo, hi):
        if s + i < n and not filled[s + i]:
            logits[s + i] = out[w, i]; filled[s + i] = True
lab = logits.argmax(-1)
t = np.arange(n) * HOP / SR

# compare on a 10 ms grid
grid = np.arange(0, min(ref_t[-1], t[-1]), 0.01)
a = ref_lab[np.clip(np.searchsorted(ref_t, grid, side="right") - 1, 0, len(ref_lab) - 1)]
b = lab[np.clip(np.searchsorted(t, grid, side="right") - 1, 0, n - 1)]
root = lambda l: np.where(l >= 168, -1, l // 14)
print(f"duration={len(y)/SR:.1f}s cqt_time={t_cqt:.2f}s windows={len(starts)}")
print(f"exact label agreement={np.mean(a == b)*100:.2f}%  root agreement={np.mean(root(a) == root(b))*100:.2f}%")
