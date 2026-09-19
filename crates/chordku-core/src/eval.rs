//! Chord recognition scoring: weighted chord symbol recall (WCSR), the MIREX measure.
//!
//! Follows `mir_eval.chord`: both annotations are cut at the union of their boundaries and
//! each piece is scored by duration. Three vocabularies:
//!
//!   root      the root pitch class matches
//!   majmin    root and major / minor triad match; reference chords that are neither
//!             (dim, aug, sus, power chords) are left out of the denominator
//!   sevenths  root and one of maj, min, maj7, min7, 7 match; other reference chords left out
//!
//! `N` (no chord) must be matched by `N`; `X` (unknown) in the reference is always left out.

use std::path::Path;

use anyhow::{bail, Context, Result};

use crate::chords::Chord;

/// A chord reduced to what the metrics compare: root and interval set (bit i = i semitones).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Label {
    NoChord,
    Unknown,
    Chord { root: u8, intervals: u16 },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Interval {
    pub t0: f64,
    pub t1: f64,
    pub label: Label,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Scores {
    pub root: f64,
    pub majmin: f64,
    pub sevenths: f64,
    /// Seconds of reference audio that counted for each measure.
    pub root_secs: f64,
    pub majmin_secs: f64,
    pub sevenths_secs: f64,
}

const fn bits(semitones: &[u8]) -> u16 {
    let mut out = 0u16;
    let mut i = 0;
    while i < semitones.len() {
        out |= 1 << (semitones[i] % 12);
        i += 1;
    }
    out
}

const MAJ: u16 = bits(&[0, 4, 7]);
const MIN: u16 = bits(&[0, 3, 7]);

/// Harte shorthand -> interval set. Unlisted shorthands fall back to "unknown quality".
fn shorthand(q: &str) -> Option<u16> {
    Some(match q {
        "" | "maj" => MAJ,
        "min" => MIN,
        "dim" => bits(&[0, 3, 6]),
        "aug" => bits(&[0, 4, 8]),
        "maj7" => bits(&[0, 4, 7, 11]),
        "min7" => bits(&[0, 3, 7, 10]),
        "7" => bits(&[0, 4, 7, 10]),
        "dim7" => bits(&[0, 3, 6, 9]),
        "hdim7" => bits(&[0, 3, 6, 10]),
        "minmaj7" => bits(&[0, 3, 7, 11]),
        "maj6" => bits(&[0, 4, 7, 9]),
        "min6" => bits(&[0, 3, 7, 9]),
        "9" => bits(&[0, 4, 7, 10, 2]),
        "maj9" => bits(&[0, 4, 7, 11, 2]),
        "min9" => bits(&[0, 3, 7, 10, 2]),
        "11" => bits(&[0, 4, 7, 10, 2, 5]),
        "min11" => bits(&[0, 3, 7, 10, 2, 5]),
        "13" => bits(&[0, 4, 7, 10, 2, 9]),
        "maj13" => bits(&[0, 4, 7, 11, 2, 9]),
        "min13" => bits(&[0, 3, 7, 10, 2, 9]),
        "sus2" => bits(&[0, 2, 7]),
        "sus4" => bits(&[0, 5, 7]),
        "1" => bits(&[0]),
        "5" => bits(&[0, 7]),
        _ => return None,
    })
}

fn root_pc(name: &str) -> Option<u8> {
    let mut chars = name.chars();
    let base = match chars.next()? {
        'C' => 0i32,
        'D' => 2,
        'E' => 4,
        'F' => 5,
        'G' => 7,
        'A' => 9,
        'B' => 11,
        _ => return None,
    };
    let shift: i32 = chars.map(|c| if c == '#' { 1 } else if c == 'b' { -1 } else { 100 }).sum();
    (shift.abs() < 12).then(|| (base + shift).rem_euclid(12) as u8)
}

/// Parse a Harte chord label (`C`, `A:min7`, `F#:maj(9)/3`, `N`, `X`).
pub fn parse_label(text: &str) -> Label {
    let text = text.trim();
    if text == "N" {
        return Label::NoChord;
    }
    let body = text.split('/').next().unwrap_or(text); // the bass note does not enter these metrics
    let (root, quality) = body.split_once(':').unwrap_or((body, ""));
    let Some(root) = root_pc(root) else { return Label::Unknown };
    // "maj(9)" -> shorthand "maj"; a bare "(1,3,5)" interval list has no shorthand.
    let short = quality.split('(').next().unwrap_or("");
    let intervals = if quality.starts_with('(') { explicit_intervals(quality) } else { shorthand(short) };
    match intervals {
        Some(intervals) => Label::Chord { root, intervals },
        None => Label::Unknown,
    }
}

fn explicit_intervals(list: &str) -> Option<u16> {
    const DEGREE: [i32; 14] = [0, 0, 2, 4, 5, 7, 9, 11, 12, 14, 16, 17, 19, 21];
    let inner = list.trim_start_matches('(').trim_end_matches(')');
    let mut out = 1u16;
    for item in inner.split(',').map(str::trim).filter(|s| !s.is_empty() && !s.starts_with('*')) {
        let flats = item.matches('b').count() as i32;
        let sharps = item.matches('#').count() as i32;
        let degree: usize = item.trim_start_matches(['b', '#']).parse().ok()?;
        let semis = DEGREE.get(degree)? + sharps - flats;
        out |= 1 << semis.rem_euclid(12);
    }
    Some(out)
}

impl From<Chord> for Label {
    fn from(c: Chord) -> Self {
        match c {
            Chord::None => Label::NoChord,
            Chord::Unknown => Label::Unknown,
            Chord::Named { root_pc, quality } => Label::Chord { root: root_pc, intervals: bits(quality.intervals()) },
        }
    }
}

fn triad(intervals: u16) -> Option<u16> {
    [MAJ, MIN].into_iter().find(|t| intervals & t == *t)
}

fn seventh(intervals: u16) -> Option<u16> {
    let t = triad(intervals)?;
    let with = |semi: u8| intervals & (1 << semi) != 0;
    Some(if t == MAJ && with(11) {
        MAJ | 1 << 11
    } else if with(10) {
        t | 1 << 10
    } else if t == MIN && with(11) {
        return None; // minmaj7 is outside the sevenths vocabulary
    } else {
        t
    })
}

/// `Some(correct)` if the reference chord counts for this measure, else `None`.
fn compare(reference: Label, estimate: Label, reduce: fn(u16) -> Option<u16>) -> Option<bool> {
    match reference {
        Label::Unknown => None,
        Label::NoChord => Some(estimate == Label::NoChord),
        Label::Chord { root, intervals } => {
            let want = reduce(intervals)?;
            Some(match estimate {
                Label::Chord { root: r, intervals: i } => r == root && reduce(i) == Some(want),
                _ => false,
            })
        }
    }
}

/// Read a `.lab` file: `start end label` per line (tabs or spaces).
pub fn read_lab(path: &Path) -> Result<Vec<Interval>> {
    let text = std::fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    let mut out = Vec::new();
    for (n, line) in text.lines().enumerate().filter(|(_, l)| !l.trim().is_empty() && !l.starts_with('#')) {
        let mut parts = line.split_whitespace();
        let (Some(a), Some(b), Some(label)) = (parts.next(), parts.next(), parts.next()) else {
            bail!("{}:{}: expected `start end label`", path.display(), n + 1);
        };
        out.push(Interval { t0: a.parse()?, t1: b.parse()?, label: parse_label(label) });
    }
    Ok(out)
}

fn label_at(intervals: &[Interval], t: f64) -> Label {
    let i = intervals.partition_point(|iv| iv.t1 <= t);
    match intervals.get(i) {
        Some(iv) if iv.t0 <= t => iv.label,
        _ => Label::NoChord,
    }
}

/// Score `estimate` against `reference` over the reference's time span.
pub fn score(reference: &[Interval], estimate: &[Interval]) -> Scores {
    let (Some(first), Some(last)) = (reference.first(), reference.last()) else { return Scores::default() };
    let mut cuts: Vec<f64> = reference
        .iter()
        .chain(estimate)
        .flat_map(|iv| [iv.t0, iv.t1])
        .filter(|t| *t >= first.t0 && *t <= last.t1)
        .collect();
    cuts.sort_by(|a, b| a.total_cmp(b));
    cuts.dedup();

    let mut s = Scores::default();
    let (mut ok_root, mut ok_majmin, mut ok_sevenths) = (0.0, 0.0, 0.0);
    for w in cuts.windows(2) {
        let dur = w[1] - w[0];
        let mid = 0.5 * (w[0] + w[1]);
        let (r, e) = (label_at(reference, mid), label_at(estimate, mid));
        if let Some(hit) = compare(r, e, |_| Some(1)) {
            s.root_secs += dur;
            ok_root += if hit { dur } else { 0.0 };
        }
        if let Some(hit) = compare(r, e, triad) {
            s.majmin_secs += dur;
            ok_majmin += if hit { dur } else { 0.0 };
        }
        if let Some(hit) = compare(r, e, seventh) {
            s.sevenths_secs += dur;
            ok_sevenths += if hit { dur } else { 0.0 };
        }
    }
    let ratio = |ok: f64, total: f64| if total > 0.0 { ok / total } else { 0.0 };
    s.root = ratio(ok_root, s.root_secs);
    s.majmin = ratio(ok_majmin, s.majmin_secs);
    s.sevenths = ratio(ok_sevenths, s.sevenths_secs);
    s
}

/// Duration-weighted mean over several recordings (the MIREX convention).
pub fn weighted_mean(all: &[Scores]) -> Scores {
    let mut out = Scores::default();
    for s in all {
        out.root += s.root * s.root_secs;
        out.majmin += s.majmin * s.majmin_secs;
        out.sevenths += s.sevenths * s.sevenths_secs;
        out.root_secs += s.root_secs;
        out.majmin_secs += s.majmin_secs;
        out.sevenths_secs += s.sevenths_secs;
    }
    out.root /= out.root_secs.max(f64::MIN_POSITIVE);
    out.majmin /= out.majmin_secs.max(f64::MIN_POSITIVE);
    out.sevenths /= out.sevenths_secs.max(f64::MIN_POSITIVE);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn iv(t0: f64, t1: f64, label: &str) -> Interval {
        Interval { t0, t1, label: parse_label(label) }
    }

    #[test]
    fn parses_harte_labels() {
        assert_eq!(parse_label("N"), Label::NoChord);
        assert_eq!(parse_label("X"), Label::Unknown);
        assert_eq!(parse_label("C"), Label::Chord { root: 0, intervals: MAJ });
        assert_eq!(parse_label("Bb:min7/b3"), Label::Chord { root: 10, intervals: bits(&[0, 3, 7, 10]) });
        assert_eq!(parse_label("F#:maj(9)"), Label::Chord { root: 6, intervals: MAJ });
        assert_eq!(parse_label("G:(1,3,5,b7)"), Label::Chord { root: 7, intervals: bits(&[0, 4, 7, 10]) });
        assert_eq!(parse_label("Cb"), Label::Chord { root: 11, intervals: MAJ });
        assert_eq!(parse_label("H:weird"), Label::Unknown);
    }

    #[test]
    fn scores_like_mir_eval() {
        // Reference: 0-2 C, 2-4 A:min7, 4-6 G:sus4, 6-8 N, 8-10 X
        let reference = [iv(0.0, 2.0, "C"), iv(2.0, 4.0, "A:min7"), iv(4.0, 6.0, "G:sus4"), iv(6.0, 8.0, "N"), iv(8.0, 10.0, "X")];
        // Estimate: C:maj7 (right triad, wrong seventh) | A:min (right triad, wrong seventh)
        //           | G (right root) | half N, half C | anything
        let estimate = [iv(0.0, 2.0, "C:maj7"), iv(2.0, 4.0, "A:min"), iv(4.0, 6.0, "G"), iv(6.0, 7.0, "N"), iv(7.0, 10.0, "C")];
        let s = score(&reference, &estimate);
        // root: X excluded -> 8 s; correct 2 + 2 + 2 + 1 = 7
        assert!((s.root_secs - 8.0).abs() < 1e-9 && (s.root - 7.0 / 8.0).abs() < 1e-9, "{s:?}");
        // majmin: X and sus4 excluded -> 6 s; correct 2 + 2 + 1 = 5
        assert!((s.majmin_secs - 6.0).abs() < 1e-9 && (s.majmin - 5.0 / 6.0).abs() < 1e-9, "{s:?}");
        // sevenths: same 6 s; C vs Cmaj7 wrong, Am7 vs Am wrong, N half right -> 1
        assert!((s.sevenths_secs - 6.0).abs() < 1e-9 && (s.sevenths - 1.0 / 6.0).abs() < 1e-9, "{s:?}");
    }

    #[test]
    fn model_classes_convert() {
        assert_eq!(Label::from(Chord::from_class(9 * 14 + 6)), parse_label("A:min7"));
        assert_eq!(Label::from(Chord::from_class(169)), Label::NoChord);
    }

    #[test]
    fn mean_is_duration_weighted() {
        let a = Scores { root: 1.0, root_secs: 30.0, majmin: 1.0, majmin_secs: 30.0, sevenths: 1.0, sevenths_secs: 30.0 };
        let b = Scores { root: 0.0, root_secs: 10.0, majmin: 0.0, majmin_secs: 10.0, sevenths: 0.0, sevenths_secs: 10.0 };
        assert!((weighted_mean(&[a, b]).root - 0.75).abs() < 1e-12);
    }
}
