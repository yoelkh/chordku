"""Decompose label disagreement into: CQT implementation vs windowing strategy."""
import sys
import librosa, numpy as np, onnxruntime as ort
from common import BPO, BTC_DIR, HOP, MODELS_OUT, N_BINS, SR, TIMESTEP
from proto_cqt import direct_cqt

path = sys.argv[1] if len(sys.argv) > 1 else str(BTC_DIR / "test" / "example.mp3")
y, _ = librosa.load(path, sr=SR, mono=True)
sess = ort.InferenceSession(str(MODELS_OUT / "btc.onnx"), providers=["CPUExecutionProvider"])
L = lambda c: np.log(np.abs(c) + 1e-6).T.astype(np.float32)
chunk = SR * 10

def chunked(fn):
    return np.concatenate([fn(y[i:i + chunk]) for i in range(0, len(y), chunk)], axis=0)

def run_blocks(feat):
    pad = (-len(feat)) % TIMESTEP
    x = np.pad(feat, ((0, pad), (0, 0))).reshape(-1, TIMESTEP, N_BINS)
    return sess.run(None, {"logcqt": x})[0].reshape(-1, 170)[: len(feat)]

def run_overlap(feat, keep_edge):
    n = len(feat); half = TIMESTEP // 2
    starts = list(range(0, max(n - TIMESTEP, 0) + 1, half))
    if starts[-1] + TIMESTEP < n: starts.append(n - TIMESTEP)
    out = sess.run(None, {"logcqt": np.stack([feat[s:s + TIMESTEP] for s in starts])})[0]
    acc = np.zeros((n, 170), np.float32); wsum = np.zeros((n, 1), np.float32)
    if keep_edge == "avg":   # triangular weighted average of overlapping windows
        w = np.minimum(np.arange(1, TIMESTEP + 1), np.arange(TIMESTEP, 0, -1)).astype(np.float32)[:, None]
    else:
        w = np.zeros((TIMESTEP, 1), np.float32); w[TIMESTEP // 4: TIMESTEP - TIMESTEP // 4] = 1
    for k, s in enumerate(starts):
        ww = w.copy()
        if k == 0: ww[: TIMESTEP // 2] = np.maximum(ww[: TIMESTEP // 2], 1e-3 if keep_edge == "avg" else 1)
        if k == len(starts) - 1: ww[TIMESTEP // 2:] = np.maximum(ww[TIMESTEP // 2:], 1e-3 if keep_edge == "avg" else 1)
        acc[s:s + TIMESTEP] += out[k] * ww; wsum[s:s + TIMESTEP] += ww
    return acc / np.maximum(wsum, 1e-9)

lib_chunk = lambda seg: L(librosa.cqt(seg, sr=SR, n_bins=N_BINS, bins_per_octave=BPO, hop_length=HOP))
dir_chunk = lambda seg: L(direct_cqt(seg))
A = run_blocks(chunked(lib_chunk))            # reference
B = run_blocks(chunked(dir_chunk))            # only the CQT changes
lib_cont = L(librosa.cqt(y, sr=SR, n_bins=N_BINS, bins_per_octave=BPO, hop_length=HOP))
dir_cont = L(direct_cqt(y))
C = run_blocks(lib_cont[: len(A)])            # continuous librosa features, reference blocks
D = run_overlap(lib_cont, "centre")
E = run_overlap(dir_cont, "centre")
F = run_overlap(dir_cont, "avg")

def cmp(name, p, q):
    n = min(len(p), len(q)); a, b = p[:n].argmax(-1), q[:n].argmax(-1)
    r = lambda l: np.where(l >= 168, -1, l // 14)
    top2 = np.argsort(-q[:n], -1)[:, :2]
    print(f"{name:46s} exact={np.mean(a == b)*100:6.2f}%  root={np.mean(r(a) == r(b))*100:6.2f}%  ref-in-top2={np.mean((top2 == a[:, None]).any(-1))*100:6.2f}%")

def conf(name, p):
    e = np.exp(p - p.max(-1, keepdims=True)); pr = e / e.sum(-1, keepdims=True)
    print(f"{name:46s} mean max-prob={pr.max(-1).mean():.4f}")

cmp("A ref  vs B (direct CQT, same chunks)", A, B)
cmp("A ref  vs C (continuous librosa, blocks)", A, C)
cmp("C      vs D (librosa cont, overlap centre)", C, D)
cmp("D      vs E (direct cont, overlap centre)", D, E)
cmp("E      vs F (direct cont, overlap avg)", E, F)
for n_, p_ in [("A", A), ("B", B), ("C", C), ("D", D), ("E", E), ("F", F)]: conf(n_, p_)
