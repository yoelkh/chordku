// Drum tab: the transcribed hits (kick / snare / hi-hat, with strengths 0-100) placed on the
// chart's bar grid as 16th-note steps. The grid comes from the chart, edits included, so a
// corrected downbeat or tempo re-quantises the tab for free.

import type { Bar } from "./layout";

export interface DrumOnset {
  t_ms: number;
  kick: number;
  snare: number;
  hat: number;
}
export interface DrumEvents {
  version: number;
  onsets: DrumOnset[];
}

/** One bar of tab: `steps` = beats x SUB, each lane holds a strength per step (0 = rest). */
export interface DrumBar {
  steps: number;
  kick: number[];
  snare: number[];
  hat: number[];
  /** Steps whose hi-hat was inferred from the bar's own pattern rather than detected. */
  hatInferred: boolean[];
}

export const SUB = 4;
/** Hits weaker than this are noise or leakage and stay off the tab. */
export const MIN_STRENGTH = 25;

/**
 * Quantise onsets onto bars. Hi-hats under a snare or kick are hard to hear apart from the
 * snare's own noise, so when a bar shows hats on most of its eighths (or sixteenths) the
 * pattern is completed at that level; those steps are flagged `hatInferred`.
 */
export function drumBars(bars: Bar[], events: DrumEvents | null | undefined, sub = SUB): DrumBar[] {
  const out: DrumBar[] = bars.map((b) => {
    const steps = b.beats.length * sub;
    return { steps, kick: new Array(steps).fill(0), snare: new Array(steps).fill(0), hat: new Array(steps).fill(0), hatInferred: new Array(steps).fill(false) };
  });
  if (!events || !bars.length) return out;

  let bi = 0;
  for (const o of events.onsets) {
    const t = o.t_ms;
    while (bi < bars.length && t >= bars[bi].t1_ms) bi++;
    if (bi >= bars.length) break;
    const bar = bars[bi];
    if (t < bar.t0_ms) continue;
    const beats = bar.beats;
    let k = 0;
    for (let i = 0; i < beats.length; i++) if (beats[i].t_ms <= t) k = i;
    const next = k + 1 < beats.length ? beats[k + 1].t_ms : bar.t1_ms;
    const frac = next > beats[k].t_ms ? (t - beats[k].t_ms) / (next - beats[k].t_ms) : 0;
    let step = k * sub + Math.round(frac * sub);
    let target = out[bi];
    if (step >= target.steps) {
      // Rounded up into the next bar's first step.
      if (bi + 1 >= bars.length) continue;
      target = out[bi + 1];
      step = 0;
    }
    if (o.kick >= MIN_STRENGTH) target.kick[step] = Math.max(target.kick[step], o.kick);
    if (o.snare >= MIN_STRENGTH) target.snare[step] = Math.max(target.snare[step], o.snare);
    if (o.hat >= MIN_STRENGTH) target.hat[step] = Math.max(target.hat[step], o.hat);
  }

  for (const bar of out) {
    // Eighths are every other 16th step; sixteenths are every step.
    for (const stride of [1, 2]) {
      const slots: number[] = [];
      for (let s = 0; s < bar.steps; s += stride) slots.push(s);
      const played = slots.filter((s) => bar.hat[s] > 0).length;
      if (played >= Math.ceil(slots.length * 0.6) && played < slots.length) {
        for (const s of slots) {
          if (bar.hat[s] === 0) {
            bar.hat[s] = MIN_STRENGTH;
            bar.hatInferred[s] = true;
          }
        }
        break;
      }
    }
  }
  return out;
}

/** Step under the playhead within a bar, or -1. */
export function stepAt(bar: Bar, ms: number, sub = SUB): number {
  if (ms < bar.t0_ms || ms >= bar.t1_ms) return -1;
  const beats = bar.beats;
  let k = 0;
  for (let i = 0; i < beats.length; i++) if (beats[i].t_ms <= ms) k = i;
  const next = k + 1 < beats.length ? beats[k + 1].t_ms : bar.t1_ms;
  const frac = next > beats[k].t_ms ? (ms - beats[k].t_ms) / (next - beats[k].t_ms) : 0;
  return Math.min(k * sub + Math.floor(frac * sub), beats.length * sub - 1);
}

/** Text form, one line per lane, for exports: `HH x-x-x-x- | ...`. */
export function drumTabText(drumBars: DrumBar[], barsPerLine = 4): string {
  const lanes: [keyof Pick<DrumBar, "hat" | "snare" | "kick">, string][] = [
    ["hat", "HH"],
    ["snare", "SD"],
    ["kick", "BD"],
  ];
  const lines: string[] = [];
  for (let i = 0; i < drumBars.length; i += barsPerLine) {
    const group = drumBars.slice(i, i + barsPerLine);
    for (const [lane, name] of lanes) {
      const cells = group.map((b) => b[lane].map((v) => (v >= 60 ? "X" : v > 0 ? "x" : "-")).join(""));
      lines.push(`${name} |${cells.join("|")}|`);
    }
    lines.push("");
  }
  return lines.join("\n");
}
