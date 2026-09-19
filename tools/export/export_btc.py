"""Export the BTC large-vocabulary chord model (MIT, jayg996/BTC-ISMIR19) to ONNX.

Graph contract (what chordku-core relies on):
  input  "logcqt"  f32 [B, 108, 144]   log(|CQT| + 1e-6), NOT normalised
  output "logits"  f32 [B, 108, 170]   pre-softmax chord scores
The checkpoint's mean/std normalisation is folded into the graph. The sequence
length is frozen at 108 so the timing signal and the causal/anti-causal bias
masks constant-fold; only the batch axis is dynamic.
"""
import argparse
import hashlib
import json
from pathlib import Path

import numpy as np
import torch
import torch.nn as nn

from common import BTC_DIR, MODELS_OUT, N_BINS, N_CHORDS, TIMESTEP, add_btc_to_path

add_btc_to_path()
from btc_model import BTC_model  # noqa: E402
import yaml  # noqa: E402


class BtcLogits(nn.Module):
    def __init__(self, model: BTC_model, mean: float, std: float):
        super().__init__()
        self.layers = model.self_attn_layers
        self.proj = model.output_layer.output_projection
        self.register_buffer("mean", torch.tensor(float(mean), dtype=torch.float32))
        self.register_buffer("std", torch.tensor(float(std), dtype=torch.float32))

    def forward(self, logcqt: torch.Tensor) -> torch.Tensor:
        x = (logcqt - self.mean) / self.std
        y, _ = self.layers(x)
        return self.proj(y)


def load(weights: str) -> BtcLogits:
    # The 2019 HParams loader calls yaml.load() without a Loader, so read the YAML directly.
    model_cfg = yaml.safe_load((BTC_DIR / "run_config.yaml").read_text())["model"]
    model_cfg["num_chords"] = N_CHORDS
    model = BTC_model(config=model_cfg)
    ckpt = torch.load(weights, map_location="cpu", weights_only=False)
    model.load_state_dict(ckpt["model"])
    mean, std = np.asarray(ckpt["mean"]).item(), np.asarray(ckpt["std"]).item()
    print(f"checkpoint mean={mean:.6f} std={std:.6f}")
    return BtcLogits(model, mean, std).eval()


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--weights", default=str(BTC_DIR / "test" / "btc_model_large_voca.pt"))
    ap.add_argument("--out", default=str(MODELS_OUT / "btc.onnx"))
    args = ap.parse_args()

    wrapped = load(args.weights)
    MODELS_OUT.mkdir(parents=True, exist_ok=True)
    dummy = torch.randn(2, TIMESTEP, N_BINS)
    torch.onnx.export(
        wrapped, (dummy,), args.out,
        input_names=["logcqt"], output_names=["logits"],
        dynamic_axes={"logcqt": {0: "batch"}, "logits": {0: "batch"}},
        opset_version=17, dynamo=False, do_constant_folding=True,
    )

    import onnxruntime as ort
    sess = ort.InferenceSession(args.out, providers=["CPUExecutionProvider"])
    worst = 0.0
    agree = total = 0
    rng = np.random.default_rng(0)
    for batch in (1, 3, 7):
        x = (rng.standard_normal((batch, TIMESTEP, N_BINS)) * 2.0 - 4.0).astype(np.float32)
        with torch.no_grad():
            ref = wrapped(torch.from_numpy(x)).numpy()
        got = sess.run(None, {"logcqt": x})[0]
        worst = max(worst, float(np.abs(ref - got).max()))
        agree += int((ref.argmax(-1) == got.argmax(-1)).sum())
        total += ref.shape[0] * ref.shape[1]
    sha = hashlib.sha256(open(args.out, "rb").read()).hexdigest()
    print(f"onnx vs torch: max|dlogit|={worst:.2e} argmax agreement={agree}/{total}")
    print(f"wrote {args.out} sha256={sha}")
    assert worst < 1e-3, "ONNX export diverges from PyTorch"

    meta = {
        "mean": float(wrapped.mean), "std": float(wrapped.std),
        "n_classes": N_CHORDS, "timestep": TIMESTEP,
        "weights": Path(args.weights).name, "sha256": sha,
        "source": "https://github.com/jayg996/BTC-ISMIR19 (MIT)",
    }
    Path(args.out).with_suffix(".json").write_text(json.dumps(meta, indent=2))


if __name__ == "__main__":
    main()
