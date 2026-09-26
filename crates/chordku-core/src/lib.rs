//! ChordKu analysis core: decoding, resampling, CQT + BTC chord inference,
//! beats, key, chart building and stem separation.
pub mod accel;
pub mod audio;
pub mod beats;
pub mod btc;
pub mod chart;
pub mod chords;
pub mod cqt;
pub mod decode;
pub mod drums;
pub mod eval;
pub mod key;
pub mod opus_enc;
pub mod ortutil;
pub mod pipeline;
pub mod resample;
pub mod sections;
pub mod separate;
pub mod tuning;
pub mod viterbi;

pub use audio::Audio;

/// Human readable ONNX Runtime build string (asserted at start-up so a stale
/// `onnxruntime.dll` on the system path is caught early).
pub fn ort_version() -> String {
    ort::info().to_string()
}
