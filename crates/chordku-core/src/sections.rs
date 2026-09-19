//! Song structure from chord repetition.
//!
//! Bars are grouped into 4-bar blocks; blocks whose chord roots match are given the
//! same letter, and runs of equal letters become one section. The result drives line
//! breaks in the chart and ready-made loop regions in the player. Users rename sections
//! ("A" -> "Verse"), so the labels only need to be consistent, not semantic.

use crate::chart::{Section, Segment};
use crate::chords::Chord;

const BARS_PER_BLOCK: usize = 4;
const SLOTS_PER_BAR: usize = 4;
const SAME_BLOCK: f32 = 0.62;
const SILENT: u16 = u16::MAX;

pub fn detect(segments: &[Segment], beats_ms: &[u32], downbeat_idx: &[u32], duration_ms: u32) -> Vec<Section> {
    let whole = |label: &str| vec![Section { t0_ms: 0, t1_ms: duration_ms, label: label.into() }];
    let mut bar_starts: Vec<u32> = downbeat_idx.iter().filter_map(|i| beats_ms.get(*i as usize).copied()).collect();
    bar_starts.dedup();
    if bar_starts.len() < 2 * BARS_PER_BLOCK || segments.is_empty() {
        return whole("A");
    }
    let mut lengths: Vec<u32> = bar_starts.windows(2).map(|w| w[1] - w[0]).collect();
    lengths.sort_unstable();
    let median = lengths[lengths.len() / 2];
    // A musical phrase lasts 8-12 s: four bars of a slow 4/4, eight bars of a fast 3/4.
    let bars_per_block = if median < 2200 && bar_starts.len() >= 4 * BARS_PER_BLOCK { 2 * BARS_PER_BLOCK } else { BARS_PER_BLOCK };
    // Close the last bar.
    bar_starts.push((bar_starts[bar_starts.len() - 1] + median).min(duration_ms));

    // Chord signature per bar: the (root, triad) sounding at the middle of each quarter.
    let bars: Vec<[u16; SLOTS_PER_BAR]> = bar_starts
        .windows(2)
        .map(|w| {
            let mut sig = [SILENT; SLOTS_PER_BAR];
            for (s, slot) in sig.iter_mut().enumerate() {
                let t = w[0] + (w[1] - w[0]) * (2 * s as u32 + 1) / (2 * SLOTS_PER_BAR as u32);
                *slot = chord_id_at(segments, t);
            }
            sig
        })
        .collect();

    // Label blocks by similarity to the first block of each existing label.
    let blocks: Vec<&[[u16; SLOTS_PER_BAR]]> = bars.chunks(bars_per_block).collect();
    let mut prototypes: Vec<usize> = Vec::new(); // block index of each label's first member
    let mut labels: Vec<usize> = Vec::with_capacity(blocks.len());
    for (i, block) in blocks.iter().enumerate() {
        let best = prototypes
            .iter()
            .enumerate()
            .map(|(label, p)| (label, similarity(blocks[*p], block)))
            .max_by(|a, b| a.1.total_cmp(&b.1));
        match best {
            Some((label, sim)) if sim >= SAME_BLOCK => labels.push(label),
            _ => {
                // A short trailing block with no match joins the previous section.
                if block.len() < bars_per_block && !labels.is_empty() {
                    labels.push(*labels.last().unwrap());
                } else {
                    prototypes.push(i);
                    labels.push(prototypes.len() - 1);
                }
            }
        }
    }

    let mut sections: Vec<Section> = Vec::new();
    for (i, label) in labels.iter().enumerate() {
        let t0 = if i == 0 { 0 } else { bar_starts[i * bars_per_block] };
        let end_bar = ((i + 1) * bars_per_block).min(bar_starts.len() - 1);
        let t1 = if i + 1 == labels.len() { duration_ms } else { bar_starts[end_bar] };
        let name = letter(*label);
        match sections.last_mut() {
            Some(last) if last.label == name => last.t1_ms = t1,
            _ => sections.push(Section { t0_ms: t0, t1_ms: t1, label: name }),
        }
    }
    sections
}

fn chord_id_at(segments: &[Segment], t: u32) -> u16 {
    let i = segments.partition_point(|s| s.t1_ms <= t);
    match segments.get(i).map(|s| Chord::from_class(s.class)) {
        // Roots only: a verse is the same verse whether the recogniser heard B:maj7 or B:maj6.
        Some(Chord::Named { root_pc, .. }) => root_pc as u16,
        _ => SILENT,
    }
}

fn similarity(a: &[[u16; SLOTS_PER_BAR]], b: &[[u16; SLOTS_PER_BAR]]) -> f32 {
    let n = a.len().min(b.len());
    if n == 0 {
        return 0.0;
    }
    let same: usize =
        a.iter().zip(b).map(|(x, y)| x.iter().zip(y).filter(|(p, q)| p == q).count()).sum();
    // Penalise comparing a full block with a truncated one.
    same as f32 / (a.len().max(b.len()) * SLOTS_PER_BAR) as f32
}

fn letter(i: usize) -> String {
    let c = (b'A' + (i % 26) as u8) as char;
    if i < 26 {
        c.to_string()
    } else {
        format!("{c}{}", i / 26 + 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn song(pattern: &[&[u16]], bar_ms: u32) -> (Vec<Segment>, Vec<u32>, Vec<u32>, u32) {
        // pattern: list of 4-bar blocks, each bar one chord class
        let mut segments = Vec::new();
        let mut t = 0;
        for block in pattern {
            for class in *block {
                segments.push(Segment {
                    t0_ms: t,
                    t1_ms: t + bar_ms,
                    class: *class,
                    root_pc: None,
                    quality: None,
                    conf: 0.9,
                    alts: vec![],
                });
                t += bar_ms;
            }
        }
        let beats: Vec<u32> = (0..t / (bar_ms / 4)).map(|i| i * bar_ms / 4).collect();
        let downs: Vec<u32> = (0..beats.len() as u32).step_by(4).collect();
        (segments, beats, downs, t)
    }

    #[test]
    fn verse_chorus_verse() {
        let verse: &[u16] = &[1, 127, 71, 99]; // C Am F G
        let chorus: &[u16] = &[71, 99, 1, 1]; // F G C C
        let (seg, beats, downs, dur) = song(&[verse, verse, chorus, verse, chorus, chorus], 2500);
        let s = detect(&seg, &beats, &downs, dur);
        let labels: Vec<&str> = s.iter().map(|x| x.label.as_str()).collect();
        assert_eq!(labels, ["A", "B", "A", "B"]);
        assert_eq!((s[0].t0_ms, s[0].t1_ms), (0, 20_000));
        assert_eq!(s.last().unwrap().t1_ms, dur);
    }

    #[test]
    fn short_bars_use_eight_bar_phrases() {
        let verse: &[u16] = &[1, 127, 71, 99];
        let chorus: &[u16] = &[71, 99, 1, 1];
        // 1.4 s bars (fast 3/4): phrases are 8 bars, so verse+verse is ONE block.
        let (seg, beats, downs, dur) = song(&[verse, verse, chorus, chorus, verse, verse, chorus, chorus], 1400);
        let s = detect(&seg, &beats, &downs, dur);
        let labels: Vec<&str> = s.iter().map(|x| x.label.as_str()).collect();
        assert_eq!(labels, ["A", "B", "A", "B"]);
        assert_eq!(s[0].t1_ms, 8 * 1400);
    }

    #[test]
    fn short_songs_are_one_section() {
        let (seg, beats, downs, dur) = song(&[&[1, 1, 1]], 2000);
        assert_eq!(detect(&seg, &beats, &downs, dur).len(), 1);
    }
}
