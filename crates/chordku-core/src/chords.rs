//! The BTC large-vocabulary chord alphabet: 12 roots x 14 qualities + X + N = 170 classes.
//!
//! Class index = `root_pc * 14 + quality`, 168 = X (unknown), 169 = N (no chord).
//! Everything downstream works in pitch classes (0 = C) and never stores a spelling;
//! spelling is a view decision made from the key (see web/src/lib/theory).

use serde::{Deserialize, Serialize};

pub const N_CLASSES: usize = 170;
pub const N_QUALITIES: usize = 14;
pub const CLASS_UNKNOWN: u16 = 168;
pub const CLASS_NO_CHORD: u16 = 169;

/// Chord qualities in BTC class order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[repr(u8)]
pub enum Quality {
    Min = 0,
    Maj = 1,
    Dim = 2,
    Aug = 3,
    Min6 = 4,
    Maj6 = 5,
    Min7 = 6,
    MinMaj7 = 7,
    Maj7 = 8,
    Dom7 = 9,
    Dim7 = 10,
    HDim7 = 11,
    Sus2 = 12,
    Sus4 = 13,
}

impl Quality {
    pub const ALL: [Quality; N_QUALITIES] = [
        Quality::Min,
        Quality::Maj,
        Quality::Dim,
        Quality::Aug,
        Quality::Min6,
        Quality::Maj6,
        Quality::Min7,
        Quality::MinMaj7,
        Quality::Maj7,
        Quality::Dom7,
        Quality::Dim7,
        Quality::HDim7,
        Quality::Sus2,
        Quality::Sus4,
    ];

    /// Harte / mir_eval shorthand.
    pub fn harte(self) -> &'static str {
        match self {
            Quality::Min => "min",
            Quality::Maj => "maj",
            Quality::Dim => "dim",
            Quality::Aug => "aug",
            Quality::Min6 => "min6",
            Quality::Maj6 => "maj6",
            Quality::Min7 => "min7",
            Quality::MinMaj7 => "minmaj7",
            Quality::Maj7 => "maj7",
            Quality::Dom7 => "7",
            Quality::Dim7 => "dim7",
            Quality::HDim7 => "hdim7",
            Quality::Sus2 => "sus2",
            Quality::Sus4 => "sus4",
        }
    }

    /// Semitone offsets from the root.
    pub fn intervals(self) -> &'static [u8] {
        match self {
            Quality::Min => &[0, 3, 7],
            Quality::Maj => &[0, 4, 7],
            Quality::Dim => &[0, 3, 6],
            Quality::Aug => &[0, 4, 8],
            Quality::Min6 => &[0, 3, 7, 9],
            Quality::Maj6 => &[0, 4, 7, 9],
            Quality::Min7 => &[0, 3, 7, 10],
            Quality::MinMaj7 => &[0, 3, 7, 11],
            Quality::Maj7 => &[0, 4, 7, 11],
            Quality::Dom7 => &[0, 4, 7, 10],
            Quality::Dim7 => &[0, 3, 6, 9],
            Quality::HDim7 => &[0, 3, 6, 10],
            Quality::Sus2 => &[0, 2, 7],
            Quality::Sus4 => &[0, 5, 7],
        }
    }

    /// The triad this quality collapses to when the chart is simplified.
    pub fn triad(self) -> Quality {
        match self {
            Quality::Min | Quality::Min6 | Quality::Min7 | Quality::MinMaj7 => Quality::Min,
            Quality::Maj | Quality::Maj6 | Quality::Maj7 | Quality::Dom7 => Quality::Maj,
            Quality::Dim | Quality::Dim7 | Quality::HDim7 => Quality::Dim,
            Quality::Aug => Quality::Aug,
            Quality::Sus2 => Quality::Sus2,
            Quality::Sus4 => Quality::Sus4,
        }
    }
}

/// A decoded model class.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Chord {
    /// Silence, noise or unpitched material.
    None,
    /// Harmonic content the vocabulary cannot name.
    Unknown,
    Named { root_pc: u8, quality: Quality },
}

impl Chord {
    pub fn from_class(class: u16) -> Chord {
        match class {
            CLASS_NO_CHORD => Chord::None,
            c if c >= CLASS_UNKNOWN => Chord::Unknown,
            c => Chord::Named {
                root_pc: (c / N_QUALITIES as u16) as u8,
                quality: Quality::ALL[(c % N_QUALITIES as u16) as usize],
            },
        }
    }

    pub fn class(self) -> u16 {
        match self {
            Chord::None => CLASS_NO_CHORD,
            Chord::Unknown => CLASS_UNKNOWN,
            Chord::Named { root_pc, quality } => root_pc as u16 * N_QUALITIES as u16 + quality as u16,
        }
    }

    /// Harte label with sharps, as written by BTC / read by mir_eval (`C#:min7`, `N`, `X`).
    pub fn harte(self) -> String {
        const ROOTS: [&str; 12] = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];
        match self {
            Chord::None => "N".into(),
            Chord::Unknown => "X".into(),
            Chord::Named { root_pc, quality: Quality::Maj } => ROOTS[root_pc as usize % 12].into(),
            Chord::Named { root_pc, quality } => format!("{}:{}", ROOTS[root_pc as usize % 12], quality.harte()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn class_round_trip() {
        for c in 0..N_CLASSES as u16 {
            assert_eq!(Chord::from_class(c).class(), c);
        }
        assert_eq!(Chord::from_class(1).harte(), "C");
        assert_eq!(Chord::from_class(14 + 6).harte(), "C#:min7");
        assert_eq!(Chord::from_class(9 * 14).harte(), "A:min");
        assert_eq!(Chord::from_class(169).harte(), "N");
    }
}
