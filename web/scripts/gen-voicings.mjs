// Trim @tombatossals/chords-db (MIT) down to the 14 chord qualities ChordKu recognises.
// Output: src/lib/diagrams/voicings.json  { guitar: { "<pc>:<quality>": [[frets, fingers, baseFret, barres], ...] }, ukulele: {...} }
import { readFileSync, writeFileSync } from "node:fs";

const QUALITY_TO_SUFFIX = {
  min: "minor", maj: "major", dim: "dim", aug: "aug", min6: "m6", maj6: "6", min7: "m7",
  minmaj7: "mmaj7", maj7: "maj7", "7": "7", dim7: "dim7", hdim7: "m7b5", sus2: "sus2", sus4: "sus4",
};
const KEY_TO_PC = { C: 0, "C#": 1, Csharp: 1, Db: 1, D: 2, Eb: 3, "D#": 3, E: 4, F: 5, "F#": 6, Fsharp: 6, Gb: 6, G: 7, Ab: 8, "G#": 8, A: 9, Bb: 10, "A#": 10, B: 11 };

const out = {};
for (const instrument of ["guitar", "ukulele"]) {
  const db = JSON.parse(readFileSync(new URL(`../node_modules/@tombatossals/chords-db/lib/${instrument}.json`, import.meta.url)));
  const table = {};
  for (const [key, chords] of Object.entries(db.chords)) {
    const pc = KEY_TO_PC[key];
    if (pc === undefined) throw new Error(`unknown key ${key}`);
    for (const [quality, suffix] of Object.entries(QUALITY_TO_SUFFIX)) {
      const chord = chords.find((c) => c.suffix === suffix);
      if (!chord) continue;
      table[`${pc}:${quality}`] = chord.positions.slice(0, 3).map((p) => [p.frets, p.fingers, p.baseFret, p.barres ?? []]);
    }
  }
  out[instrument] = table;
  const missing = [];
  for (let pc = 0; pc < 12; pc++) for (const q of Object.keys(QUALITY_TO_SUFFIX)) if (!table[`${pc}:${q}`]) missing.push(`${pc}:${q}`);
  console.log(instrument, Object.keys(table).length, "chords; missing:", missing.join(" ") || "none");
}
writeFileSync(new URL("../src/lib/diagrams/voicings.json", import.meta.url), JSON.stringify(out));
