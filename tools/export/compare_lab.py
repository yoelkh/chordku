"""Agreement between two .lab files on a 10 ms grid:  python compare_lab.py ref.lab est.lab"""
import sys
import numpy as np

ROOTS = {"C": 0, "C#": 1, "D": 2, "D#": 3, "E": 4, "F": 5, "F#": 6, "G": 7, "G#": 8, "A": 9, "A#": 10, "B": 11}
MIN_LIKE = {"min", "min6", "min7", "minmaj7", "dim", "dim7", "hdim7"}

def load(path):
    rows = [l.split() for l in open(path) if l.strip()]
    return np.array([[float(r[0]), float(r[1])] for r in rows]), [r[2] for r in rows]

def sample(iv, labels, grid):
    idx = np.clip(np.searchsorted(iv[:, 0], grid, side="right") - 1, 0, len(labels) - 1)
    return [labels[i] for i in idx]

def root(l): return l.split(":")[0]
def majmin(l):
    if l in ("N", "X"): return l
    r, _, q = l.partition(":")
    return r + (":min" if q in MIN_LIKE else "")

ref_iv, ref_l = load(sys.argv[1]); est_iv, est_l = load(sys.argv[2])
if len(sys.argv) > 3:  # est was computed on audio that starts `offset` seconds later
    est_iv = est_iv + float(sys.argv[3])
grid = np.arange(max(ref_iv[0, 0], est_iv[0, 0]), min(ref_iv[-1, 1], est_iv[-1, 1]), 0.01)
a, b = sample(ref_iv, ref_l, grid), sample(est_iv, est_l, grid)
for name, f in (("exact", lambda x: x), ("majmin", majmin), ("root", root)):
    print(f"{name:7s} {np.mean([f(x) == f(y) for x, y in zip(a, b)]) * 100:6.2f}%")
