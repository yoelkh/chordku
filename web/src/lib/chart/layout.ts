// Turn the effective chart into what a musician reads: sections -> lines of bars -> beats,
// with a chord printed on the beat where it starts.

import type { Chart, Section } from "./model";
import { segmentAt } from "./model";

export interface BeatCell {
  t_ms: number;
  /** Segment index that starts on (or is first heard at) this beat, else -1. */
  segment: number;
  /** Segment sounding during this beat (for highlighting and editing). */
  sounding: number;
}
export interface Bar {
  index: number;
  t0_ms: number;
  t1_ms: number;
  beats: BeatCell[];
}
export interface SectionBlock {
  section: Section;
  bars: Bar[];
}

export const BARS_PER_LINE = 4;

/** Bars from the beat grid. Returns [] when the recording has no usable grid. */
export function buildBars(chart: Chart): Bar[] {
  const { beats_ms: beats, downbeat_idx: downs, segments, duration_ms } = chart;
  if (beats.length < 8 || downs.length < 2) return [];

  // Bar boundaries as beat indices, including a pickup bar before the first downbeat.
  const starts = downs[0] > 0 ? [0, ...downs] : downs.slice();
  const bars: Bar[] = [];
  let lastSegment = -1;
  for (let b = 0; b < starts.length; b++) {
    const from = starts[b];
    const to = b + 1 < starts.length ? starts[b + 1] : beats.length;
    const t0 = b === 0 ? Math.min(beats[from], 0) : beats[from];
    const t1 = to < beats.length ? beats[to] : duration_ms;
    const cells: BeatCell[] = [];
    for (let i = from; i < to; i++) {
      const next = i + 1 < beats.length ? beats[i + 1] : duration_ms;
      // Sample a little inside the beat: chord boundaries sit exactly on beat times.
      const sounding = segmentAt(segments, Math.min(beats[i] + 40, next - 1));
      const starting = sounding !== lastSegment ? sounding : -1;
      if (sounding !== -1) lastSegment = sounding;
      cells.push({ t_ms: beats[i], segment: starting, sounding });
    }
    if (cells.length) bars.push({ index: bars.length, t0_ms: t0, t1_ms: t1, beats: cells });
  }
  return bars;
}

export function groupBySection(chart: Chart, bars: Bar[]): SectionBlock[] {
  const sections: Section[] = chart.sections.length
    ? chart.sections
    : [{ t0_ms: 0, t1_ms: chart.duration_ms, label: "A" }];
  const blocks: SectionBlock[] = sections.map((section) => ({ section, bars: [] }));
  for (const bar of bars) {
    const mid = (bar.t0_ms + bar.t1_ms) / 2;
    const block = blocks.find((b) => mid >= b.section.t0_ms && mid < b.section.t1_ms) ?? blocks[blocks.length - 1];
    block.bars.push(bar);
  }
  return blocks.filter((b) => b.bars.length);
}

export function lines<T>(items: T[], perLine = BARS_PER_LINE): T[][] {
  const out: T[][] = [];
  for (let i = 0; i < items.length; i += perLine) out.push(items.slice(i, i + perLine));
  return out;
}
