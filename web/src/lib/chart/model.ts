// Chart data: the server's immutable analysis, the user's edits layered on top, and the
// resulting "effective" chart every view renders from. Edits are keyed by time range so
// they survive a re-analysis.

import { CLASS_NO_CHORD, type Key, type Mode } from "../theory";

export interface Alt {
  class: number;
  p: number;
}
export interface Segment {
  t0_ms: number;
  t1_ms: number;
  class: number;
  conf: number;
  alts: Alt[];
  /** True when the user set this chord. */
  edited?: boolean;
}
export interface Section {
  t0_ms: number;
  t1_ms: number;
  label: string;
}
export interface KeySegment {
  t0_ms: number;
  tonic_pc: number;
  mode: Mode;
}
export interface Analysis {
  pipeline_ver: number;
  duration_ms: number;
  tuning_cents: number;
  bpm: number | null;
  meter: number;
  beats_ms: number[];
  downbeat_idx: number[];
  key: (Key & { confidence: number }) | null;
  key_segments: KeySegment[];
  segments: Segment[];
  sections: Section[];
}

export type EditOp =
  | "set_chord"
  | "delete"
  | "split"
  | "key_override"
  | "meter_override"
  | "downbeat_shift"
  | "tempo_scale"
  | "section_rename";

export interface Edit {
  id: number;
  t0_ms: number;
  t1_ms: number;
  op: EditOp;
  payload: Record<string, unknown>;
}

export interface Chart {
  duration_ms: number;
  bpm: number | null;
  meter: number;
  tuning_cents: number;
  beats_ms: number[];
  downbeat_idx: number[];
  key: Key | null;
  key_segments: KeySegment[];
  segments: Segment[];
  sections: Section[];
}

/** Replace `[t0, t1)` with one chord, splitting whatever it overlaps. */
function overwrite(segments: Segment[], t0: number, t1: number, cls: number): Segment[] {
  if (t1 <= t0) return segments;
  const out: Segment[] = [];
  let placed = false;
  const place = () => {
    if (!placed) {
      out.push({ t0_ms: t0, t1_ms: t1, class: cls, conf: 1, alts: [], edited: true });
      placed = true;
    }
  };
  for (const s of segments) {
    if (s.t1_ms <= t0) {
      out.push(s);
    } else if (s.t0_ms >= t1) {
      place();
      out.push(s);
    } else {
      if (s.t0_ms < t0) out.push({ ...s, t1_ms: t0 });
      place();
      if (s.t1_ms > t1) out.push({ ...s, t0_ms: t1 });
    }
  }
  place();
  return out;
}

function mergeEqual(segments: Segment[]): Segment[] {
  const out: Segment[] = [];
  for (const s of segments) {
    const last = out[out.length - 1];
    if (last && last.class === s.class && last.t1_ms === s.t0_ms && !!last.edited === !!s.edited) {
      const w0 = last.t1_ms - last.t0_ms;
      const w1 = s.t1_ms - s.t0_ms;
      last.conf = (last.conf * w0 + s.conf * w1) / Math.max(1, w0 + w1);
      last.t1_ms = s.t1_ms;
    } else {
      out.push({ ...s });
    }
  }
  return out;
}

/** Double (factor 2) or halve (0.5) the beat grid, for songs tracked at the wrong metrical level. */
function scaleBeats(beats: number[], downbeats: number[], factor: number): { beats: number[]; downbeats: number[] } {
  if (factor === 2) {
    const out: number[] = [];
    const map: number[] = [];
    beats.forEach((b, i) => {
      map[i] = out.length;
      out.push(b);
      if (i + 1 < beats.length) out.push(Math.round((b + beats[i + 1]) / 2));
    });
    return { beats: out, downbeats: downbeats.map((d) => map[d]) };
  }
  if (factor === 0.5) {
    const first = downbeats.length ? downbeats[0] % 2 : 0;
    const keep = beats.map((_, i) => i).filter((i) => i % 2 === first);
    const index = new Map(keep.map((i, k) => [i, k]));
    return { beats: keep.map((i) => beats[i]), downbeats: downbeats.filter((d) => index.has(d)).map((d) => index.get(d)!) };
  }
  return { beats, downbeats };
}

/** Rebuild downbeats every `meter` beats, starting from the first detected downbeat plus `shift`. */
function regrid(beats: number[], downbeats: number[], meter: number, shift: number): number[] {
  if (!beats.length || meter < 2) return downbeats;
  const anchor = (((downbeats[0] ?? 0) + shift) % meter + meter) % meter;
  const out: number[] = [];
  for (let i = anchor; i < beats.length; i += meter) out.push(i);
  return out;
}

export function applyEdits(analysis: Analysis, edits: Edit[]): Chart {
  let segments: Segment[] = analysis.segments.map((s) => ({ ...s }));
  let sections = analysis.sections.map((s) => ({ ...s }));
  let beats = analysis.beats_ms.slice();
  let downbeats = analysis.downbeat_idx.slice();
  let meter = analysis.meter;
  let bpm = analysis.bpm;
  let key: Key | null = analysis.key ? { tonic_pc: analysis.key.tonic_pc, mode: analysis.key.mode } : null;
  let keySegments = analysis.key_segments.slice();
  let shift = 0;
  let regridNeeded = false;

  for (const e of [...edits].sort((a, b) => a.id - b.id)) {
    const p = e.payload ?? {};
    switch (e.op) {
      case "set_chord":
        if (typeof p.class === "number") segments = overwrite(segments, e.t0_ms, e.t1_ms, p.class);
        break;
      case "delete":
        segments = overwrite(segments, e.t0_ms, e.t1_ms, CLASS_NO_CHORD);
        break;
      case "split": {
        const at = typeof p.at_ms === "number" ? p.at_ms : e.t0_ms;
        segments = segments.flatMap((s) =>
          at > s.t0_ms && at < s.t1_ms ? [{ ...s, t1_ms: at }, { ...s, t0_ms: at, edited: true }] : [s],
        );
        break;
      }
      case "key_override":
        if (typeof p.tonic_pc === "number" && (p.mode === "major" || p.mode === "minor")) {
          key = { tonic_pc: p.tonic_pc, mode: p.mode };
          keySegments = [{ t0_ms: 0, tonic_pc: p.tonic_pc, mode: p.mode }];
        }
        break;
      case "meter_override":
        if (typeof p.meter === "number" && p.meter >= 2 && p.meter <= 12) {
          meter = p.meter;
          regridNeeded = true;
        }
        break;
      case "downbeat_shift":
        if (typeof p.shift === "number") {
          shift += p.shift;
          regridNeeded = true;
        }
        break;
      case "tempo_scale":
        if (p.factor === 2 || p.factor === 0.5) {
          ({ beats, downbeats } = scaleBeats(beats, downbeats, p.factor));
          if (bpm) bpm = bpm * p.factor;
          regridNeeded = true;
        }
        break;
      case "section_rename":
        if (typeof p.label === "string") {
          const s = sections.find((x) => e.t0_ms >= x.t0_ms && e.t0_ms < x.t1_ms);
          if (s) s.label = p.label.slice(0, 40);
        }
        break;
    }
  }
  if (regridNeeded) downbeats = regrid(beats, downbeats, meter, shift);

  return {
    duration_ms: analysis.duration_ms,
    bpm,
    meter,
    tuning_cents: analysis.tuning_cents,
    beats_ms: beats,
    downbeat_idx: downbeats,
    key,
    key_segments: keySegments,
    segments: mergeEqual(segments),
    sections,
  };
}

/** Index of the segment sounding at `ms` (binary search), or -1. */
export function segmentAt(segments: Segment[], ms: number): number {
  let lo = 0;
  let hi = segments.length - 1;
  while (lo <= hi) {
    const mid = (lo + hi) >> 1;
    if (ms < segments[mid].t0_ms) hi = mid - 1;
    else if (ms >= segments[mid].t1_ms) lo = mid + 1;
    else return mid;
  }
  return -1;
}

/** The key in force at `ms`. */
export function keyAt(chart: Chart, ms: number): Key | null {
  let current: Key | null = chart.key;
  for (const k of chart.key_segments) {
    if (k.t0_ms <= ms) current = { tonic_pc: k.tonic_pc, mode: k.mode };
  }
  return current;
}
