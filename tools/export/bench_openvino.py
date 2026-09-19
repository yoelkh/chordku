"""Feasibility study: ChordKu's three networks on Intel NPU / Arc iGPU / CPU through OpenVINO.
Static shapes (the NPU requires them). Prints compile time, warm inference time and the
deviation from the OpenVINO CPU result."""
import sys, time
import numpy as np
import openvino as ov

M = "../../models/"
core = ov.Core()
rng = np.random.default_rng(0)
golden = np.fromfile("../../testdata/golden/synth_logcqt_lib.f32", dtype="<f4").reshape(1, 108, 144)

CASES = {
    "btc x16": (M + "btc.onnx", {"logcqt": np.repeat(golden, 16, 0) + rng.normal(0, 0.05, (16, 108, 144)).astype(np.float32)}),
    "beat 1500": (M + "beat_this_small.onnx", {"spectrogram": rng.normal(0, 1, (1, 1500, 128)).astype(np.float32)}),
    "htdemucs": (M + "htdemucs_fwd.onnx", {"x": rng.normal(0, 1, (1, 4, 2048, 336)).astype(np.float32), "xt": rng.normal(0, 1, (1, 2, 343980)).astype(np.float32)}),
}
only = sys.argv[1:] or list(CASES)
for name in only:
    path, feeds = CASES[name]
    model = core.read_model(path)
    model.reshape({k: list(v.shape) for k, v in feeds.items()})
    ref = None
    for dev in ["CPU", "NPU", "GPU.0"]:
        try:
            t = time.time(); cm = core.compile_model(model, dev); tc = time.time() - t
            req = cm.create_infer_request()
            req.infer(feeds)
            runs = []
            for _ in range(3):
                t = time.time(); out = req.infer(feeds); runs.append(time.time() - t)
            first = out[cm.output(0)]
            if ref is None: ref = first
            diff = float(np.abs(first - ref).max()); scale = float(np.abs(ref).max())
            agree = float((first.argmax(-1) == ref.argmax(-1)).mean()) if first.ndim == 3 and first.shape[-1] == 170 else float("nan")
            print(f"{name:10s} {dev:6s} compile {tc:6.1f}s  infer {min(runs):7.3f}s  max|d| {diff:9.2e} (ref max {scale:.1f})  argmax agree {agree:.4f}", flush=True)
        except Exception as e:
            print(f"{name:10s} {dev:6s} FAILED: {str(e).splitlines()[0][:150]}", flush=True)
