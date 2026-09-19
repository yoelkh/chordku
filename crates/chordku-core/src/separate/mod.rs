//! Stem separation: drums / bass / other / vocals.
//!
//! The [`Separator`] trait keeps the rest of the application independent of the model
//! behind it, so a faster 2-stem model or a GPU back end can be dropped in later.

use std::ops::ControlFlow;

use anyhow::Result;
use serde::{Deserialize, Serialize};

pub mod htdemucs;
pub mod remix;
pub mod stft;

pub const N_STEMS: usize = 4;
/// All separators work on 44.1 kHz stereo.
pub const STEM_RATE: u32 = 44_100;

/// Stems in model output order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Stem {
    Drums = 0,
    Bass = 1,
    Other = 2,
    Vocals = 3,
}

impl Stem {
    pub const ALL: [Stem; N_STEMS] = [Stem::Drums, Stem::Bass, Stem::Other, Stem::Vocals];

    pub fn name(self) -> &'static str {
        match self {
            Stem::Drums => "drums",
            Stem::Bass => "bass",
            Stem::Other => "other",
            Stem::Vocals => "vocals",
        }
    }
}

/// A run of finished output samples.
pub struct StemBlock<'a> {
    /// Position of the first sample in the track.
    pub offset: usize,
    /// `[stem][channel] -> samples`, indexed by `Stem as usize`.
    pub stems: &'a [[Vec<f32>; 2]],
    /// 1-based index of the segment that completed this block, and the total.
    pub segment: usize,
    pub segments: usize,
}

pub trait Separator {
    /// Separate planar 44.1 kHz stereo. Blocks arrive in order and tile the track exactly.
    /// The sink may return `Break` to cancel; the result is `false` in that case.
    fn separate(
        &mut self,
        mix: &[Vec<f32>; 2],
        sink: &mut dyn FnMut(StemBlock<'_>) -> Result<ControlFlow<()>>,
    ) -> Result<bool>;
}
