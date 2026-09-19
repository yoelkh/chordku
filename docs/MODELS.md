# Models

All inference is ONNX Runtime 1.28 (CPU) through the `ort` crate, statically linked.

| File | What | Size | License of the weights | Origin |
|---|---|---|---|---|
| `btc.onnx` + `btc.json` | BTC large-vocabulary chord recogniser, 170 classes | 12.9 MB | MIT | exported locally from jayg996/BTC-ISMIR19 |
| `mel_spectrogram.onnx` | log-mel front end of Beat This! | 0.3 MB | MIT | danigb/beat-this-rs |
| `beat_this_small.onnx` | Beat This! small, beats and downbeats | 10.6 MB | MIT | danigb/beat-this-rs |
| `htdemucs_fwd.onnx` + `.data` | Hybrid Transformer Demucs, network only | 171 MB | research / personal use only | Intel/demucs-openvino |

Search order for model files: `CHORDKU_MODELS`, `<exe>/models`, `./models`, `<data>/models`.
Downloads started from the app go to `<data>/models` and are verified by size and SHA-256
(`crates/chordku-server/src/models.rs`).

## Why these models

Chord recognition candidates were screened for open weights, a permissive license and a front end
that can be reproduced exactly:

| Candidate | Verdict |
|---|---|
| BTC (ISMIR 2019) | **chosen**: MIT code and weights, pure PyTorch ops, plain `librosa.cqt` front end, 170 classes |
| ChordMini BTC-PL | same architecture with newer weights; drop-in through `export_btc.py --weights` once evaluated |
| Chord-CNN-LSTM / lv-chordia | MIT, good, but a 5-model ensemble with `hybrid_cqt` and an HMM: about three times the porting work |
| consonance-ACE (2025) | MIT, worth benchmarking later |
| madmom | model weights CC BY-NC-SA: excluded |
| Chordino / NNLS-Chroma, autochord | GPL (directly or through its feature extractor): excluded |
| Essentia ChordsDetection | AGPL and template based: excluded |
| ChordFormer (2026) | no public weights |

Beat tracking: Beat This! needs no DBN post-processing, which is what makes it portable, and its
small model keeps most of the accuracy at a tenth of the size. madmom's beat models are CC BY-NC-SA.

Separation: htdemucs is the best quality that still runs on a laptop CPU (RTF 0.38 here). Its
authors offer the weights for scientific purposes only, so ChordKu never bundles or rehosts them:
the user downloads them from Intel's repository from inside the app, for personal use. RoFormer
models are 2-3 dB better but far too heavy for CPU. A faster natively-ONNX 2-stem model can be
added behind the `Separator` trait.

## Exporting BTC

```
tools\setup.ps1          # or, manually:
cd tools\export
python -m venv .venv && .venv\Scripts\pip install -r requirements.lock
git clone --depth 1 https://github.com/jayg996/BTC-ISMIR19 _vendor\btc
.venv\Scripts\python export_btc.py [--weights other_checkpoint.pt]
```

Graph contract: input `logcqt` f32 `[B, 108, 144]` = `ln(|CQT| + 1e-6)`, not normalised (the
checkpoint's mean and std are folded into the graph); output `logits` `[B, 108, 170]`. The sequence
length is frozen at 108 so the timing signal and the causal masks constant-fold. The export is
checked against PyTorch on random batches (max logit difference 1.5e-5, identical argmax).

## Parity tests

| What | Reference | Result |
|---|---|---|
| CQT vs numpy twin of the same algorithm | `make_golden.py` | max 0.003 on bins above -2 (log units) |
| CQT vs `librosa.cqt` 0.11 | golden + `proto_decomp.py` | mean 0.0001, std 0.008 on energetic bins; 99.3 % identical BTC labels on music |
| BTC ONNX vs PyTorch | golden logits | < 1e-3 |
| htdemucs DSP vs PyTorch Demucs | `ref_demucs.py` | 73.5-100 dB SNR per stem |
| Beat This! | synthetic 100 BPM pattern | tempo within 1.5 BPM, every beat within 40 ms, 4/4 |
