// Music theory for the chart views. The server stores chords at concert pitch as pitch
// classes (0 = C) with no spelling; everything here is a pure view transform:
//   sounding = analysed + transpose        (the player pitch-shifts by the same amount)
//   shape    = sounding - capo             (what a guitarist fingers)
// Spelling is chosen from the key the result is displayed in.

export const QUALITIES = [
  "min", "maj", "dim", "aug", "min6", "maj6", "min7", "minmaj7", "maj7", "7", "dim7", "hdim7", "sus2", "sus4",
] as const;
export type Quality = (typeof QUALITIES)[number];
export type Mode = "major" | "minor";

export const CLASS_UNKNOWN = 168;
export const CLASS_NO_CHORD = 169;

export interface Chord {
  root: number;
  quality: Quality;
}
export interface Key {
  tonic_pc: number;
  mode: Mode;
}

const SUFFIX: Record<Quality, string> = {
  min: "m", maj: "", dim: "dim", aug: "aug", min6: "m6", maj6: "6", min7: "m7", minmaj7: "m(maj7)",
  maj7: "maj7", "7": "7", dim7: "dim7", hdim7: "m7b5", sus2: "sus2", sus4: "sus4",
};
const INTERVALS: Record<Quality, number[]> = {
  min: [0, 3, 7], maj: [0, 4, 7], dim: [0, 3, 6], aug: [0, 4, 8], min6: [0, 3, 7, 9], maj6: [0, 4, 7, 9],
  min7: [0, 3, 7, 10], minmaj7: [0, 3, 7, 11], maj7: [0, 4, 7, 11], "7": [0, 4, 7, 10], dim7: [0, 3, 6, 9],
  hdim7: [0, 3, 6, 10], sus2: [0, 2, 7], sus4: [0, 5, 7],
};
const SHARPS = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];
const FLATS = ["C", "Db", "D", "Eb", "E", "F", "Gb", "G", "Ab", "A", "Bb", "B"];

export const mod12 = (n: number) => ((n % 12) + 12) % 12;

export function classToChord(cls: number): Chord | null {
  if (cls < 0 || cls >= CLASS_UNKNOWN) return null;
  return { root: Math.floor(cls / 14), quality: QUALITIES[cls % 14] };
}

export function chordToClass(chord: Chord): number {
  return chord.root * 14 + QUALITIES.indexOf(chord.quality);
}

export function chordTones(chord: Chord): number[] {
  return INTERVALS[chord.quality].map((iv) => mod12(chord.root + iv));
}

/** Major keys with flats in their signature: F Bb Eb Ab Db (and Gb is spelled F#). */
const FLAT_MAJORS = new Set([5, 10, 3, 8, 1]);

export function prefersFlats(key: Key | null): boolean {
  if (!key) return false;
  const major = key.mode === "minor" ? mod12(key.tonic_pc + 3) : key.tonic_pc;
  return FLAT_MAJORS.has(major);
}

export function noteName(pc: number, flats: boolean): string {
  return (flats ? FLATS : SHARPS)[mod12(pc)];
}

export function keyName(key: Key): string {
  return noteName(key.tonic_pc, prefersFlats(key)) + (key.mode === "minor" ? "m" : "");
}

export function transposeKey(key: Key, semitones: number): Key {
  return { tonic_pc: mod12(key.tonic_pc + semitones), mode: key.mode };
}

export function transposeChord(chord: Chord, semitones: number): Chord {
  return { root: mod12(chord.root + semitones), quality: chord.quality };
}

/** 0 = as recognised, 1 = drop colour tones but keep 7 / m7 / sus / dim, 2 = triads only. */
export type SimplifyLevel = 0 | 1 | 2;

export function simplify(chord: Chord, level: SimplifyLevel): Chord {
  if (level === 0) return chord;
  const q = chord.quality;
  let out: Quality = q;
  if (level === 1) {
    if (q === "min6" || q === "minmaj7") out = "min";
    else if (q === "maj6" || q === "maj7") out = "maj";
    else if (q === "dim7" || q === "hdim7") out = "dim";
  } else {
    if (q === "min6" || q === "min7" || q === "minmaj7") out = "min";
    else if (q === "maj6" || q === "maj7" || q === "7" || q === "sus2" || q === "sus4") out = "maj";
    else if (q === "dim7" || q === "hdim7") out = "dim";
  }
  return { root: chord.root, quality: out };
}

export function chordName(chord: Chord, flats: boolean): string {
  return noteName(chord.root, flats) + SUFFIX[chord.quality];
}

export function qualitySuffix(q: Quality): string {
  return SUFFIX[q];
}

// ---- Nashville numbers ------------------------------------------------------------------

const DEGREES = ["1", "b2", "2", "b3", "3", "4", "b5", "5", "b6", "6", "b7", "7"];

/** Numbers are relative to the major key; a minor-key song is numbered from its relative major (tonic = 6m). */
export function nashville(chord: Chord, key: Key): { degree: string; suffix: string } {
  const major = key.mode === "minor" ? mod12(key.tonic_pc + 3) : key.tonic_pc;
  return { degree: DEGREES[mod12(chord.root - major)], suffix: SUFFIX[chord.quality] };
}

// ---- Capo suggestion --------------------------------------------------------------------

const easy = (names: string[]) => new Set(names);
// Shapes by how they sit under the hand, keyed by sharp-spelled chord names.
const OPEN = easy(["C", "A", "G", "E", "D", "Am", "Em", "Dm"]);
const NEAR_OPEN = easy([
  "A7", "B7", "C7", "D7", "E7", "G7", "Am7", "Em7", "Dm7", "Cmaj7", "Fmaj7", "Amaj7", "Dmaj7", "Gmaj7",
  "Asus2", "Asus4", "Dsus2", "Dsus4", "Esus4", "Gsus4", "Csus2", "A6", "D6", "E6", "G6", "C6", "Am6", "Em6", "Dm6",
]);
const SMALL_BARRE = easy(["F", "Bm", "F#m", "B", "Bm7", "F#m7", "Bb"]);

export function shapeDifficulty(chord: Chord): number {
  const name = chordName(chord, false);
  if (OPEN.has(name)) return 0;
  if (NEAR_OPEN.has(name)) return 1;
  if (SMALL_BARRE.has(chordName(chord, false)) || SMALL_BARRE.has(chordName(chord, true))) return 2;
  if (chord.quality === "dim7" || chord.quality === "aug" || chord.quality === "minmaj7" || chord.quality === "hdim7") return 4;
  return 3;
}

export interface CapoOption {
  capo: number;
  /** Mean shape difficulty weighted by how long each chord sounds (0 = all open chords). */
  score: number;
  /** Key the guitarist thinks in with this capo. */
  shapeKey: Key | null;
  /** The most used shapes, longest first. */
  shapes: string[];
}

/** Rank capo positions 0..7 for a song given its sounding chords and their durations. */
export function suggestCapo(chords: { chord: Chord; ms: number }[], key: Key | null): CapoOption[] {
  const total = chords.reduce((s, c) => s + c.ms, 0);
  if (total <= 0) return [];
  const options: CapoOption[] = [];
  for (let capo = 0; capo <= 7; capo++) {
    const shapeKey = key ? transposeKey(key, -capo) : null;
    const flats = prefersFlats(shapeKey);
    const usage = new Map<string, number>();
    let cost = 0;
    for (const { chord, ms } of chords) {
      const shape = transposeChord(chord, -capo);
      cost += ms * shapeDifficulty(shape);
      const name = chordName(shape, flats);
      usage.set(name, (usage.get(name) ?? 0) + ms);
    }
    // High capos shorten the neck and thin the tone: small penalty, steeper past the 5th fret.
    const score = cost / total + 0.08 * capo + (capo > 5 ? 0.15 * (capo - 5) : 0);
    const shapes = [...usage.entries()].sort((a, b) => b[1] - a[1]).slice(0, 6).map(([n]) => n);
    options.push({ capo, score, shapeKey, shapes });
  }
  return options.sort((a, b) => a.score - b.score || a.capo - b.capo);
}

/** Keep the displayed transposition in -6..+6: beyond that the other direction reads easier. */
export function wrapTranspose(semitones: number): number {
  let t = Math.round(semitones) % 12;
  if (t > 6) t -= 12;
  if (t < -6) t += 12;
  return t;
}
