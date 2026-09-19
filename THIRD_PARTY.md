# Third-party work ChordKu builds on

ChordKu's own code is MIT (see `LICENSE`). It stands on the following work.

## Models

| Model | Authors | Code | Weights | How ChordKu gets it |
|---|---|---|---|---|
| BTC, bi-directional transformer for chord recognition | Park, Choi, Jeon, Kim, Park (ISMIR 2019) | MIT | MIT (in the repository) | exported to ONNX locally by `tools/export/export_btc.py` |
| Beat This! beat and downbeat tracker | Foscarin, Schlueter, Widmer (ISMIR 2024) | MIT | MIT | ONNX files from the MIT Rust port danigb/beat-this-rs, downloaded by the app |
| Hybrid Transformer Demucs (htdemucs) | Rouard, Massa, Defossez (ICASSP 2023) | MIT | provided by the authors "for scientific purposes"; **not MIT** | ONNX export by Intel (huggingface.co/Intel/demucs-openvino), downloaded by the user from inside the app, never redistributed by ChordKu |

The chunking and peak picking in `crates/chordku-core/src/beats.rs` follow beat-this-rs (MIT).
The DSP around htdemucs in `crates/chordku-core/src/separate/` re-implements `demucs.htdemucs`,
`demucs.apply` and `demucs.separate` (MIT). The CQT reproduces `librosa.cqt` (ISC).

## Data and tools

- Guitar and ukulele fingerings: @tombatossals/chords-db (MIT), trimmed by `web/scripts/gen-voicings.mjs`.
- Time-stretch / pitch-shift: Signalsmith Stretch (MIT).
- Audio fetching: yt-dlp (Unlicense), downloaded from its official release and run as a separate program.
- Evaluation data: GuitarSet (Xi et al., ISMIR 2018), CC BY 4.0; used for measurements only, not shipped.

Rust and JavaScript dependencies keep their own licenses; none is copyleft. Deliberately avoided:
madmom models (CC BY-NC-SA), Chordino / NNLS-Chroma (GPL), Essentia (AGPL), Rubber Band (GPL),
the `yt-dlp` crate (GPL-3.0), `mp3lame-encoder` (LGPL).
