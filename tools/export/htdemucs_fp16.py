"""Reference half-precision conversion of the htdemucs graph (onnxconverter-common).

The app does this itself, in Rust (crates/chordku-core/src/separate/half.rs); this script is
the reference it was checked against: both give the same fidelity on the same audio
(`chordku-cli bench song --only stems --devices cpu,cuda --stems-variant <out>/htdemucs_fwd.onnx`).

  python htdemucs_fp16.py <models>/htdemucs_fwd.onnx <out-dir> [--keep-norm-fp32]

Inputs and outputs stay float32, so the Rust side does not change. The weights are never
redistributed: this runs on the file the user downloaded.
"""
import argparse
from pathlib import Path

import onnx
from onnxconverter_common import float16


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("src")
    ap.add_argument("out_dir")
    ap.add_argument("--keep-norm-fp32", action="store_true", help="leave normalisation and softmax in float32")
    args = ap.parse_args()

    model = onnx.load(args.src)  # pulls in the external .data file next to it
    block = list(float16.DEFAULT_OP_BLOCK_LIST)
    if args.keep_norm_fp32:
        block += ["InstanceNormalization", "LayerNormalization", "Softmax"]
    half = float16.convert_float_to_float16(model, keep_io_types=True, op_block_list=block, disable_shape_infer=True)

    out = Path(args.out_dir)
    out.mkdir(parents=True, exist_ok=True)
    dst = out / "htdemucs_fwd.onnx"
    onnx.save(half, str(dst), save_as_external_data=True, all_tensors_to_one_file=True, location="htdemucs_fwd.onnx.data")
    size = (out / "htdemucs_fwd.onnx.data").stat().st_size / 1e6
    print(f"wrote {dst} (+ {size:.0f} MB of weights)")


if __name__ == "__main__":
    main()
