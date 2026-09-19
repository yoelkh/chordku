//! Harmonic re-weighting for the optional "refine chords with stems" pass.
//!
//! Feeding bare stems to a chord recogniser makes it worse (separation artefacts are out
//! of distribution); keeping the full mix but turning vocals and drums down and the
//! pitched accompaniment up has been shown to help BTC (Mitoma & Furuya, APSIPA 2025).
//! Whether it helps on a given collection is decided by measurement (`chordku-cli eval`),
//! not assumed.

use super::{Stem, N_STEMS};

/// Linear gains per stem, indexed by `Stem as usize`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RemixGains(pub [f32; N_STEMS]);

impl RemixGains {
    /// Drums and vocals -9 dB, bass unchanged, other +3 dB.
    pub const HARMONIC: RemixGains = RemixGains([0.355, 1.0, 1.413, 0.355]);

    pub fn gain(&self, stem: Stem) -> f32 {
        self.0[stem as usize]
    }
}

/// Mix one block of stems down to mono with the given gains, appending to `out`.
pub fn mono_block(stems: &[[Vec<f32>; 2]], gains: RemixGains, out: &mut Vec<f32>) {
    let len = stems.first().map(|s| s[0].len()).unwrap_or(0);
    out.reserve(len);
    for n in 0..len {
        let mut v = 0f32;
        for (s, pair) in stems.iter().enumerate().take(N_STEMS) {
            v += gains.0[s] * 0.5 * (pair[0][n] + pair[1][n]);
        }
        out.push(v);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unity_gains_rebuild_the_mono_mix() {
        let stems: Vec<[Vec<f32>; 2]> = (0..4).map(|s| [vec![s as f32; 3], vec![s as f32 + 1.0; 3]]).collect();
        let mut out = Vec::new();
        mono_block(&stems, RemixGains([1.0; 4]), &mut out);
        assert_eq!(out, vec![8.0; 3]); // (0+1+1+2+2+3+3+4)/2
    }
}
