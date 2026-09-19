"""Shared helpers for the one-time model export scripts (dev time only, never at runtime)."""
import json
import sys
from pathlib import Path

import numpy as np

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent.parent
VENDOR = HERE / "_vendor"
BTC_DIR = VENDOR / "btc"
MODELS_OUT = ROOT / "models"
GOLDEN = ROOT / "testdata" / "golden"
AUDIO = ROOT / "testdata" / "audio"

SR = 22050
HOP = 2048
N_BINS = 144
BPO = 24
TIMESTEP = 108
N_CHORDS = 170

# numpy >= 1.24 removed np.float, which the 2019 BTC code still uses.
if not hasattr(np, "float"):
    np.float = float  # type: ignore[attr-defined]


def add_btc_to_path():
    if not BTC_DIR.exists():
        sys.exit(f"missing {BTC_DIR}; run: git clone --depth 1 https://github.com/jayg996/BTC-ISMIR19 {BTC_DIR}")
    sys.path.insert(0, str(BTC_DIR))


def save_f32(path: Path, arr: np.ndarray, manifest: dict):
    """Raw little-endian f32 + shape in the manifest: trivial to read from Rust."""
    arr = np.ascontiguousarray(arr, dtype="<f4")
    path.parent.mkdir(parents=True, exist_ok=True)
    arr.tofile(path)
    manifest[path.name] = {"shape": list(arr.shape), "dtype": "f32le"}


def write_manifest(path: Path, manifest: dict):
    path.write_text(json.dumps(manifest, indent=2, sort_keys=True))
