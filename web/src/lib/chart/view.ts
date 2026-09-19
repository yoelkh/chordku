// Per-song display preferences and the single function that turns a recognised chord class
// into what is printed, given those preferences.

import {
  CLASS_NO_CHORD,
  chordName,
  classToChord,
  keyName,
  nashville,
  prefersFlats,
  simplify,
  transposeChord,
  transposeKey,
  type Chord,
  type Key,
  type SimplifyLevel,
} from "../theory";
import { keyAt, type Chart } from "./model";

export type Notation = "names" | "nashville";
export type Instrument = "guitar" | "ukulele" | "piano";

export interface ViewPrefs {
  /** Semitones the song is moved, -6..+6. The player pitch-shifts along when `pitch_follow`. */
  transpose: number;
  /** Capo fret: changes the printed shapes only. */
  capo: number;
  simplify: SimplifyLevel;
  notation: Notation;
  instrument: Instrument;
  pitch_follow: boolean;
}

export const DEFAULT_PREFS: ViewPrefs = {
  transpose: 0,
  capo: 0,
  simplify: 0,
  notation: "names",
  instrument: "guitar",
  pitch_follow: true,
};

export function normalizePrefs(raw: unknown): ViewPrefs {
  const p = (raw && typeof raw === "object" ? raw : {}) as Partial<ViewPrefs>;
  const int = (v: unknown, lo: number, hi: number, d: number) =>
    typeof v === "number" && Number.isFinite(v) ? Math.max(lo, Math.min(hi, Math.round(v))) : d;
  return {
    transpose: int(p.transpose, -6, 6, 0),
    capo: int(p.capo, 0, 11, 0),
    simplify: int(p.simplify, 0, 2, 0) as SimplifyLevel,
    notation: p.notation === "nashville" ? "nashville" : "names",
    instrument: p.instrument === "ukulele" || p.instrument === "piano" ? p.instrument : "guitar",
    pitch_follow: p.pitch_follow !== false,
  };
}

export interface ChordLabel {
  /** Main text: "F#m7", "6m7", "N.C." ... */
  text: string;
  /** The chord as fingered (after transpose, capo and simplification), for diagrams. */
  shape: Chord | null;
  /** The chord as it sounds (after transpose), for the piano diagram and audition. */
  sounding: Chord | null;
}

const NO_CHORD: ChordLabel = { text: "N.C.", shape: null, sounding: null };
const UNKNOWN: ChordLabel = { text: "?", shape: null, sounding: null };

/** Key the printed names are spelled in. */
export function displayKey(key: Key | null, prefs: ViewPrefs): Key | null {
  return key ? transposeKey(key, prefs.transpose - prefs.capo) : null;
}

export function soundingKey(key: Key | null, prefs: ViewPrefs): Key | null {
  return key ? transposeKey(key, prefs.transpose) : null;
}

export function labelFor(cls: number, key: Key | null, prefs: ViewPrefs): ChordLabel {
  if (cls === CLASS_NO_CHORD) return NO_CHORD;
  const recognised = classToChord(cls);
  if (!recognised) return UNKNOWN;
  const base = simplify(recognised, prefs.simplify);
  const sounding = transposeChord(base, prefs.transpose);
  const shape = transposeChord(sounding, -prefs.capo);
  if (prefs.notation === "nashville" && key) {
    const n = nashville(base, key); // numbers do not change under transposition
    return { text: n.degree + n.suffix, shape, sounding };
  }
  return { text: chordName(shape, prefersFlats(displayKey(key, prefs))), shape, sounding };
}

export function labelAt(chart: Chart, segmentIndex: number, prefs: ViewPrefs): ChordLabel {
  const s = chart.segments[segmentIndex];
  return s ? labelFor(s.class, keyAt(chart, s.t0_ms), prefs) : NO_CHORD;
}

/** "G (capo 2, terdengar A)" style summary of the key under the current preferences. */
export function keySummary(chart: Chart, prefs: ViewPrefs): { shown: string; sounding: string } | null {
  if (!chart.key) return null;
  return { shown: keyName(displayKey(chart.key, prefs)!), sounding: keyName(soundingKey(chart.key, prefs)!) };
}
