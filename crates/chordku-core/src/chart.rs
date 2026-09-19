//! From frame-wise chord posteriors and a beat grid to the chart the user reads.
//!
//! 1. Pool the frame log-probabilities inside every beat (overlap weighted).
//! 2. Decode one chord per beat with the sticky Viterbi; changing chord is cheap on a
//!    downbeat, dearer on other beats, which snaps changes onto the bar grid.
//! 3. Merge equal neighbours into segments, keeping a confidence and the best
//!    alternatives so a wrong chord can be fixed with one tap.
//!
//! Everything is stored at concert pitch as pitch classes. Spelling, transposition, capo,
//! simplification and Nashville numbers are view transforms done by the client.

use serde::{Deserialize, Serialize};

use crate::beats::BeatGrid;
use crate::btc::ChordPosteriors;
use crate::chords::{Chord, Quality, CLASS_NO_CHORD, N_CLASSES};
use crate::key::{self, Key, KeySegment, TimedChord};
use crate::viterbi;

/// Bump when the analysis output changes meaning, so stored charts can be re-analysed.
pub const PIPELINE_VERSION: u32 = 1;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Alt {
    /// BTC class index (see `chords`).
    pub class: u16,
    /// Mean posterior probability over the segment.
    pub p: f32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Segment {
    pub t0_ms: u32,
    pub t1_ms: u32,
    /// BTC class index; 169 = no chord, 168 = unknown.
    pub class: u16,
    /// `None` for N / X.
    pub root_pc: Option<u8>,
    pub quality: Option<Quality>,
    /// Mean posterior probability of `class` over the segment.
    pub conf: f32,
    /// Best other candidates, most likely first.
    pub alts: Vec<Alt>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Section {
    pub t0_ms: u32,
    pub t1_ms: u32,
    /// "A", "B", ... in order of first appearance; equal letters are repeats.
    pub label: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Analysis {
    pub pipeline_ver: u32,
    pub duration_ms: u32,
    /// Offset from A440 that was corrected before chord recognition.
    pub tuning_cents: f32,
    pub bpm: Option<f32>,
    /// Beats per bar.
    pub meter: u8,
    pub beats_ms: Vec<u32>,
    /// Indices into `beats_ms` that start a bar.
    pub downbeat_idx: Vec<u32>,
    pub key: Option<Key>,
    pub key_segments: Vec<KeySegment>,
    pub segments: Vec<Segment>,
    pub sections: Vec<Section>,
}

/// Log-domain costs of changing chord at a downbeat / on any other beat / per frame.
const SWITCH_DOWNBEAT: f32 = 1.2;
const SWITCH_BEAT: f32 = 3.0;
const SWITCH_FRAME: f32 = 4.0;
/// Below this many beats the grid is not trusted and we decode frame by frame.
const MIN_BEATS: usize = 8;

struct Step {
    t0: f64,
    t1: f64,
    downbeat: bool,
}

/// Build the chart. `time_scale` converts posterior time to real time (it differs from 1
/// when the analysis audio was pitch-corrected by resampling).
pub fn build(post: &ChordPosteriors, time_scale: f64, grid: &BeatGrid, duration: f64, tuning_cents: f32) -> Analysis {
    let frame = post.frame_secs * time_scale;
    let steps = steps_from(grid, duration, frame, post.frames);
    let n = steps.len();

    // 1. pool log-probabilities per step
    let mut emissions = vec![0f32; n * N_CLASSES];
    let mut probs = vec![0f32; n * N_CLASSES];
    for (s, step) in steps.iter().enumerate() {
        let e = &mut emissions[s * N_CLASSES..(s + 1) * N_CLASSES];
        let p = &mut probs[s * N_CLASSES..(s + 1) * N_CLASSES];
        let first = ((step.t0 / frame - 0.5).floor().max(0.0)) as usize;
        let last = (((step.t1 / frame + 0.5).ceil()) as usize).min(post.frames);
        let mut total = 0f32;
        for t in first..last {
            // frame t represents [t - 0.5, t + 0.5) * frame
            let lo = ((t as f64 - 0.5) * frame).max(step.t0);
            let hi = ((t as f64 + 0.5) * frame).min(step.t1);
            let w = (hi - lo) as f32;
            if w <= 0.0 {
                continue;
            }
            for ((ev, pv), lp) in e.iter_mut().zip(p.iter_mut()).zip(post.row(t)) {
                *ev += w * lp;
                *pv += w * lp.exp();
            }
            total += w;
        }
        if total > 0.0 {
            // Emission is scaled to "frames of evidence" so switch costs mean the same
            // thing at any tempo; probabilities become a plain mean.
            let inv = 1.0 / total;
            let frames_of_evidence = (total / frame as f32).max(0.5);
            e.iter_mut().for_each(|v| *v = *v * inv * frames_of_evidence);
            p.iter_mut().for_each(|v| *v *= inv);
        } else {
            e.fill(0.0);
            p.fill(1.0 / N_CLASSES as f32);
        }
    }

    // 2. decode
    let beat_sync = grid.beats.len() >= MIN_BEATS;
    let cost: Vec<f32> = steps
        .iter()
        .map(|s| if !beat_sync { SWITCH_FRAME } else if s.downbeat { SWITCH_DOWNBEAT } else { SWITCH_BEAT })
        .collect();
    let path = viterbi::decode(&emissions, N_CLASSES, &cost);

    // 3. merge into segments
    let mut segments: Vec<Segment> = Vec::new();
    let mut start = 0usize;
    for s in 1..=n {
        if s < n && path[s] == path[start] {
            continue;
        }
        let class = path[start];
        let mut mean = vec![0f32; N_CLASSES];
        let mut dur = 0f32;
        for (k, step) in steps.iter().enumerate().take(s).skip(start) {
            let w = (step.t1 - step.t0) as f32;
            for (m, p) in mean.iter_mut().zip(&probs[k * N_CLASSES..(k + 1) * N_CLASSES]) {
                *m += w * p;
            }
            dur += w;
        }
        mean.iter_mut().for_each(|m| *m /= dur.max(1e-9));
        let mut order: Vec<u16> = (0..N_CLASSES as u16).filter(|c| *c != class).collect();
        order.sort_by(|a, b| mean[*b as usize].total_cmp(&mean[*a as usize]));
        let alts = order.iter().take(3).map(|c| Alt { class: *c, p: round3(mean[*c as usize]) }).collect();
        let (root_pc, quality) = match Chord::from_class(class) {
            Chord::Named { root_pc, quality } => (Some(root_pc), Some(quality)),
            _ => (None, None),
        };
        segments.push(Segment {
            t0_ms: ms(steps[start].t0),
            t1_ms: ms(steps[s - 1].t1),
            class,
            root_pc,
            quality,
            conf: round3(mean[class as usize]),
            alts,
        });
        start = s;
    }
    absorb_blips(&mut segments);

    // key, meter, sections
    let timed: Vec<TimedChord> = segments
        .iter()
        .map(|s| TimedChord { t0: s.t0_ms as f64 / 1e3, t1: s.t1_ms as f64 / 1e3, chord: Chord::from_class(s.class) })
        .collect();
    let key = key::estimate(&timed);
    let key_segments = key.map(|k| key::segments(&timed, k, duration)).unwrap_or_default();

    let beats_ms: Vec<u32> = grid.beats.iter().map(|b| ms(*b)).collect();
    let mut downbeat_idx = Vec::with_capacity(grid.downbeats.len());
    for d in &grid.downbeats {
        if let Some(i) = grid.beats.iter().position(|b| (b - d).abs() < 1e-6) {
            downbeat_idx.push(i as u32);
        }
    }
    let meter = grid.beats_per_bar().unwrap_or(4) as u8;
    let sections = crate::sections::detect(&segments, &beats_ms, &downbeat_idx, ms(duration));

    Analysis {
        pipeline_ver: PIPELINE_VERSION,
        duration_ms: ms(duration),
        tuning_cents,
        bpm: grid.bpm().map(|b| (b * 10.0).round() as f32 / 10.0),
        meter,
        beats_ms,
        downbeat_idx,
        key,
        key_segments,
        segments,
        sections,
    }
}

fn steps_from(grid: &BeatGrid, duration: f64, frame: f64, frames: usize) -> Vec<Step> {
    if grid.beats.len() < MIN_BEATS {
        return (0..frames)
            .map(|t| Step {
                t0: ((t as f64 - 0.5) * frame).max(0.0),
                t1: ((t as f64 + 0.5) * frame).min(duration.max(frame)),
                downbeat: false,
            })
            .filter(|s| s.t1 > s.t0)
            .collect();
    }
    let is_down = |b: f64| grid.downbeats.iter().any(|d| (d - b).abs() < 1e-6);
    let mut steps = Vec::with_capacity(grid.beats.len() + 2);
    // Lead-in before the first beat, split on the extrapolated grid so a long intro
    // without detected beats is not forced to be a single chord.
    let first = grid.beats[0];
    let ibi0 = (grid.beats[1] - grid.beats[0]).max(0.2);
    let mut t = first - ((first / ibi0).floor() * ibi0);
    let mut prev = 0.0;
    while t < first - 1e-6 {
        if t - prev > 0.05 {
            steps.push(Step { t0: prev, t1: t, downbeat: false });
            prev = t;
        }
        t += ibi0;
    }
    if first - prev > 0.05 {
        steps.push(Step { t0: prev, t1: first, downbeat: false });
    }
    for (i, b) in grid.beats.iter().enumerate() {
        let next = grid.beats.get(i + 1).copied();
        match next {
            Some(n) => steps.push(Step { t0: *b, t1: n, downbeat: is_down(*b) }),
            None => {
                // Tail after the last beat, again on the extrapolated grid.
                let ibi = (b - grid.beats[i - 1]).max(0.2);
                let mut t0 = *b;
                let mut down = is_down(*b);
                while t0 < duration - 0.05 {
                    let t1 = (t0 + ibi).min(duration);
                    steps.push(Step { t0, t1, downbeat: down });
                    down = false;
                    t0 = t1;
                }
            }
        }
    }
    steps
}

/// A one-beat chord squeezed between two equal neighbours with low confidence is almost
/// always a passing note or a recognition flicker; fold it into its surroundings.
fn absorb_blips(segments: &mut Vec<Segment>) {
    let mut i = 1;
    while i + 1 < segments.len() {
        let (prev, cur, next) = (&segments[i - 1], &segments[i], &segments[i + 1]);
        let short = cur.t1_ms - cur.t0_ms < 700;
        if short && prev.class == next.class && cur.conf < 0.45 && prev.class != CLASS_NO_CHORD {
            let t1 = next.t1_ms;
            let w0 = (prev.t1_ms - prev.t0_ms) as f32;
            let w2 = (next.t1_ms - next.t0_ms) as f32;
            let conf = round3((prev.conf * w0 + next.conf * w2) / (w0 + w2).max(1.0));
            segments[i - 1].t1_ms = t1;
            segments[i - 1].conf = conf;
            segments.drain(i..=i + 1);
        } else {
            i += 1;
        }
    }
}

fn ms(t: f64) -> u32 {
    (t.max(0.0) * 1000.0).round() as u32
}

fn round3(v: f32) -> f32 {
    (v * 1000.0).round() / 1000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Posteriors that say `class_at(t)` with probability 0.9.
    fn fake_post(frames: usize, class_at: impl Fn(f64) -> u16) -> ChordPosteriors {
        let frame_secs = 2048.0 / 22_050.0;
        let mut log_probs = vec![(0.1f32 / 169.0).ln(); frames * N_CLASSES];
        for t in 0..frames {
            log_probs[t * N_CLASSES + class_at(t as f64 * frame_secs) as usize] = 0.9f32.ln();
        }
        ChordPosteriors { frames, log_probs, frame_secs }
    }

    fn grid(bpm: f64, secs: f64) -> BeatGrid {
        let ibi = 60.0 / bpm;
        let beats: Vec<f64> = (0..).map(|i| 0.25 + i as f64 * ibi).take_while(|b| *b < secs).collect();
        let downbeats = beats.iter().step_by(4).copied().collect();
        BeatGrid { beats, downbeats }
    }

    #[test]
    fn changes_snap_to_downbeats() {
        // True changes every 2 s starting at 0.25 s (= every bar at 120 BPM), but the
        // posteriors switch 150 ms late, as recognisers do. The chart must still change
        // exactly on the downbeats.
        let classes = [1u16, 9 * 14, 5 * 14 + 1, 7 * 14 + 1];
        let post = fake_post(260, |t| classes[(((t - 0.25 - 0.15).max(0.0)) / 2.0) as usize % 4]);
        let a = build(&post, 1.0, &grid(120.0, 24.0), 24.0, 0.0);
        assert_eq!(a.meter, 4);
        assert_eq!(a.bpm, Some(120.0));
        let named: Vec<&Segment> = a.segments.iter().filter(|s| s.t1_ms > 300).collect();
        for s in &named[1..named.len() - 1] {
            let bar_phase = (s.t0_ms as i64 - 250).rem_euclid(2000);
            assert_eq!(bar_phase, 0, "segment starts at {} ms", s.t0_ms);
            assert_eq!(s.t1_ms - s.t0_ms, 2000);
        }
        assert_eq!(a.key.map(|k| k.tonic_pc), Some(0));
        assert!(a.segments.iter().all(|s| s.alts.len() == 3));
    }

    #[test]
    fn falls_back_to_frames_without_beats() {
        let post = fake_post(130, |t| if t < 6.0 { 1 } else { 9 * 14 });
        let a = build(&post, 1.0, &BeatGrid::default(), 12.0, 0.0);
        assert_eq!(a.segments.len(), 2);
        assert!((a.segments[0].t1_ms as i64 - 6000).abs() < 150, "{:?}", a.segments[0]);
        assert!(a.beats_ms.is_empty());
    }

    #[test]
    fn time_scale_stretches_the_chart() {
        let post = fake_post(130, |t| if t < 6.0 { 1 } else { 9 * 14 });
        let a = build(&post, 0.5, &BeatGrid::default(), 6.0, 25.0);
        assert!((a.segments[0].t1_ms as i64 - 3000).abs() < 100);
        assert_eq!(a.tuning_cents, 25.0);
    }
}
