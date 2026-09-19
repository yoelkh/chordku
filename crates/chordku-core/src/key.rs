//! Key estimation from the chord sequence itself.
//!
//! The chart is what the user reads, so the key must be consistent with it: we score the
//! 24 major / minor keys by how well the duration-weighted chords fit each key's harmonic
//! functions. A second, windowed pass finds sustained modulations (the last-chorus "truck
//! driver" key change is everywhere in pop), smoothed with the sticky Viterbi decoder so
//! that a few borrowed chords never register as a key change.

use serde::{Deserialize, Serialize};

use crate::chords::{Chord, Quality};
use crate::viterbi;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    Major,
    Minor,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Key {
    pub tonic_pc: u8,
    pub mode: Mode,
    /// Margin over the runner-up, `0..=1`.
    pub confidence: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct KeySegment {
    pub t0_ms: u32,
    pub tonic_pc: u8,
    pub mode: Mode,
}

/// A chord with the time it sounds, as the key estimator needs it.
#[derive(Clone, Copy, Debug)]
pub struct TimedChord {
    pub t0: f64,
    pub t1: f64,
    pub chord: Chord,
}

const MAJOR_SCALE: [bool; 12] = [true, false, true, false, true, true, false, true, false, true, false, true];
const MINOR_SCALE: [bool; 12] = [true, false, true, true, false, true, false, true, true, false, true, false];

/// How strongly a chord on scale degree `deg` (semitones above the tonic) with the given
/// triad supports the key. Negative for chords that fight it.
fn fit(mode: Mode, deg: u8, quality: Quality) -> f32 {
    use Quality::*;
    let triad = quality.triad();
    let majorish = matches!(triad, Maj | Sus2 | Sus4 | Aug);
    let minorish = matches!(triad, Min);
    let dimish = matches!(triad, Dim);
    let functional = match mode {
        Mode::Major => match deg {
            0 if majorish => 1.5,
            5 if majorish => 0.9,
            7 if majorish => 1.0,
            9 if minorish => 0.8,
            2 if minorish => 0.7,
            4 if minorish => 0.6,
            11 if dimish => 0.4,
            _ => 0.0,
        },
        Mode::Minor => match deg {
            0 if minorish => 1.5,
            5 if minorish => 0.9,
            7 if majorish => 0.9, // harmonic-minor dominant
            7 if minorish => 0.6,
            8 if majorish => 0.8,
            10 if majorish => 0.7,
            3 if majorish => 0.7,
            2 if dimish => 0.4,
            _ => 0.0,
        },
    };
    if functional > 0.0 {
        // A dominant seventh on V is the strongest single key cue there is.
        return if deg == 7 && quality == Dom7 { functional + 0.3 } else { functional };
    }
    let scale = if mode == Mode::Major { &MAJOR_SCALE } else { &MINOR_SCALE };
    let tones = quality.intervals();
    let outside = tones.iter().filter(|iv| !scale[((deg + **iv) % 12) as usize]).count();
    if outside == 0 {
        0.2
    } else {
        -0.6 * outside as f32 / tones.len() as f32
    }
}

fn key_index(tonic: u8, mode: Mode) -> usize {
    tonic as usize + if mode == Mode::Minor { 12 } else { 0 }
}

fn key_of(index: usize) -> (u8, Mode) {
    ((index % 12) as u8, if index >= 12 { Mode::Minor } else { Mode::Major })
}

/// Scores of all 24 keys for the chords overlapping `[from, to)`.
fn scores(chords: &[TimedChord], from: f64, to: f64) -> [f32; 24] {
    let mut out = [0f32; 24];
    for c in chords {
        let Chord::Named { root_pc, quality } = c.chord else { continue };
        let dur = (c.t1.min(to) - c.t0.max(from)) as f32;
        if dur <= 0.0 {
            continue;
        }
        for (k, slot) in out.iter_mut().enumerate() {
            let (tonic, mode) = key_of(k);
            *slot += dur * fit(mode, (root_pc + 12 - tonic) % 12, quality);
        }
    }
    out
}

/// The global key, or `None` when the recording has no named chords.
pub fn estimate(chords: &[TimedChord]) -> Option<Key> {
    let named: Vec<&TimedChord> = chords.iter().filter(|c| matches!(c.chord, Chord::Named { .. })).collect();
    let (first, last) = (named.first()?, named.last()?);
    let total: f64 = named.iter().map(|c| c.t1 - c.t0).sum();
    let mut s = scores(chords, f64::NEG_INFINITY, f64::INFINITY);

    // Songs overwhelmingly end, and usually begin, on the tonic: this is what separates a
    // key from its relative major / minor, which share every diatonic chord.
    for (c, weight) in [(first, 0.04), (last, 0.08)] {
        if let Chord::Named { root_pc, quality } = c.chord {
            let mode = if quality.triad() == Quality::Min { Mode::Minor } else { Mode::Major };
            s[key_index(root_pc, mode)] += (total * weight) as f32 * 1.5;
        }
    }

    let mut order: Vec<usize> = (0..24).collect();
    order.sort_by(|a, b| s[*b].total_cmp(&s[*a]));
    let (best, second) = (s[order[0]], s[order[1]]);
    if best <= 0.0 {
        return None;
    }
    let (tonic_pc, mode) = key_of(order[0]);
    Some(Key { tonic_pc, mode, confidence: ((best - second) / best).clamp(0.0, 1.0) })
}

/// Key over time. Always starts with the global key at 0 ms; further entries are
/// modulations that hold for a substantial stretch.
pub fn segments(chords: &[TimedChord], global: Key, duration: f64) -> Vec<KeySegment> {
    const WINDOW: f64 = 24.0;
    const STEP: f64 = 6.0;
    let mut out = vec![KeySegment { t0_ms: 0, tonic_pc: global.tonic_pc, mode: global.mode }];
    let steps = ((duration - WINDOW) / STEP).floor() as i64 + 1;
    if steps < 3 {
        return out;
    }
    let steps = steps as usize;
    let mut emissions = Vec::with_capacity(steps * 24);
    for w in 0..steps {
        let from = w as f64 * STEP;
        let mut s = scores(chords, from, from + WINDOW);
        // Bias towards the global key so that only clear evidence moves away from it.
        s[key_index(global.tonic_pc, global.mode)] += 0.15 * WINDOW as f32;
        let norm = WINDOW as f32 / 4.0;
        emissions.extend(s.iter().map(|v| v / norm));
    }
    let path = viterbi::decode(&emissions, 24, &vec![2.5; steps]);

    let mut current = key_index(global.tonic_pc, global.mode) as u16;
    for (w, &k) in path.iter().enumerate() {
        if k == current {
            continue;
        }
        // A window votes for the stretch around its centre; place the change at the first
        // chord boundary after the point where the new key starts to dominate.
        let pivot = w as f64 * STEP + WINDOW / 2.0 - STEP / 2.0;
        let t = chords
            .iter()
            .map(|c| c.t0)
            .filter(|t0| *t0 >= pivot - STEP)
            .min_by(|a, b| (a - pivot).abs().total_cmp(&(b - pivot).abs()))
            .unwrap_or(pivot);
        let (tonic_pc, mode) = key_of(k as usize);
        out.push(KeySegment { t0_ms: (t * 1000.0).round() as u32, tonic_pc, mode });
        current = k;
    }
    // Relative major/minor flips share a key signature and are not worth a new segment.
    out.dedup_by(|b, a| relative(a, b));
    out
}

fn relative(a: &KeySegment, b: &KeySegment) -> bool {
    let sig = |k: &KeySegment| if k.mode == Mode::Minor { (k.tonic_pc + 3) % 12 } else { k.tonic_pc };
    sig(a) == sig(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seq(roots: &[(u8, Quality)], bar: f64, offset: f64) -> Vec<TimedChord> {
        roots
            .iter()
            .enumerate()
            .map(|(i, (r, q))| TimedChord {
                t0: offset + i as f64 * bar,
                t1: offset + (i + 1) as f64 * bar,
                chord: Chord::Named { root_pc: *r, quality: *q },
            })
            .collect()
    }

    #[test]
    fn major_key_from_i_vi_iv_v() {
        use Quality::*;
        let prog = [(7, Maj), (4, Min), (0, Maj), (2, Maj), (7, Maj), (4, Min), (0, Maj), (2, Dom7), (7, Maj)];
        let key = estimate(&seq(&prog, 2.0, 0.0)).unwrap();
        assert_eq!((key.tonic_pc, key.mode), (7, Mode::Major)); // G major
    }

    #[test]
    fn minor_key_with_major_dominant() {
        use Quality::*;
        let prog = [(9, Min), (5, Maj), (0, Maj), (4, Dom7), (9, Min), (2, Min), (4, Dom7), (9, Min)];
        let key = estimate(&seq(&prog, 2.0, 0.0)).unwrap();
        assert_eq!((key.tonic_pc, key.mode), (9, Mode::Minor)); // A minor
    }

    #[test]
    fn detects_a_whole_tone_modulation() {
        use Quality::*;
        let base = [(0u8, Maj), (9, Min), (5, Maj), (7, Maj)];
        let mut chords = Vec::new();
        for rep in 0..12 {
            chords.extend(seq(&base, 2.0, rep as f64 * 8.0));
        }
        let up: Vec<(u8, Quality)> = base.iter().map(|(r, q)| ((r + 2) % 12, *q)).collect();
        for rep in 0..8 {
            chords.extend(seq(&up, 2.0, 96.0 + rep as f64 * 8.0));
        }
        let global = estimate(&chords).unwrap();
        assert_eq!((global.tonic_pc, global.mode), (0, Mode::Major));
        let segs = segments(&chords, global, 160.0);
        assert_eq!(segs.len(), 2, "{segs:?}");
        assert_eq!((segs[1].tonic_pc, segs[1].mode), (2, Mode::Major));
        assert!((segs[1].t0_ms as f64 / 1000.0 - 96.0).abs() <= 8.0, "{segs:?}");
    }

    #[test]
    fn no_chords_no_key() {
        assert!(estimate(&[TimedChord { t0: 0.0, t1: 3.0, chord: Chord::None }]).is_none());
    }
}
