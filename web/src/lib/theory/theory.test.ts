import fc from "fast-check";
import { describe, expect, it } from "vitest";
import {
  QUALITIES,
  chordName,
  chordToClass,
  classToChord,
  keyName,
  nashville,
  prefersFlats,
  simplify,
  suggestCapo,
  transposeChord,
  transposeKey,
  wrapTranspose,
  type Chord,
  type Key,
} from "./index";

const chordArb = fc.record({ root: fc.integer({ min: 0, max: 11 }), quality: fc.constantFrom(...QUALITIES) }) as fc.Arbitrary<Chord>;
const keyArb = fc.record({ tonic_pc: fc.integer({ min: 0, max: 11 }), mode: fc.constantFrom("major", "minor") }) as fc.Arbitrary<Key>;

describe("classes", () => {
  it("round-trip through the 168 named classes", () => {
    for (let c = 0; c < 168; c++) expect(chordToClass(classToChord(c)!)).toBe(c);
    expect(classToChord(168)).toBeNull();
    expect(classToChord(169)).toBeNull();
    expect(chordName(classToChord(1)!, false)).toBe("C");
    expect(chordName(classToChord(14 + 6)!, false)).toBe("C#m7");
    expect(chordName(classToChord(9 * 14)!, false)).toBe("Am");
  });
});

describe("transposition", () => {
  it("12 semitones is the identity, and steps compose", () => {
    fc.assert(
      fc.property(chordArb, fc.integer({ min: -24, max: 24 }), fc.integer({ min: -24, max: 24 }), (c, a, b) => {
        expect(transposeChord(c, 12)).toEqual(c);
        expect(transposeChord(transposeChord(c, a), b)).toEqual(transposeChord(c, a + b));
        expect(transposeChord(transposeChord(c, a), -a)).toEqual(c);
      }),
    );
  });

  it("keeps the displayed amount within -6..+6 without changing the sound", () => {
    fc.assert(
      fc.property(fc.integer({ min: -40, max: 40 }), (t) => {
        const w = wrapTranspose(t);
        expect(w).toBeGreaterThanOrEqual(-6);
        expect(w).toBeLessThanOrEqual(6);
        expect((((w - t) % 12) + 12) % 12).toBe(0);
      }),
    );
  });

  it("Nashville numbers do not change when song and key move together", () => {
    fc.assert(
      fc.property(chordArb, keyArb, fc.integer({ min: -11, max: 11 }), (c, k, t) => {
        expect(nashville(transposeChord(c, t), transposeKey(k, t))).toEqual(nashville(c, k));
      }),
    );
  });
});

describe("spelling follows the key", () => {
  it("uses flats in flat keys and sharps in sharp keys", () => {
    expect(keyName({ tonic_pc: 10, mode: "major" })).toBe("Bb");
    expect(keyName({ tonic_pc: 6, mode: "major" })).toBe("F#");
    expect(keyName({ tonic_pc: 1, mode: "major" })).toBe("Db");
    expect(keyName({ tonic_pc: 7, mode: "minor" })).toBe("Gm"); // relative Bb
    expect(keyName({ tonic_pc: 8, mode: "minor" })).toBe("G#m"); // relative B
    expect(prefersFlats({ tonic_pc: 2, mode: "minor" })).toBe(true); // Dm, relative F
    expect(prefersFlats(null)).toBe(false);
    // Eb major: the IV chord is Ab, never G#.
    expect(chordName({ root: 8, quality: "maj" }, prefersFlats({ tonic_pc: 3, mode: "major" }))).toBe("Ab");
    // E major: the iii chord is G#m, never Abm.
    expect(chordName({ root: 8, quality: "min" }, prefersFlats({ tonic_pc: 4, mode: "major" }))).toBe("G#m");
  });

  it("never prints impossible names", () => {
    fc.assert(
      fc.property(chordArb, fc.boolean(), (c, flats) => {
        expect(chordName(c, flats)).toMatch(/^[A-G][#b]?(m|dim|aug|sus|maj|6|7|\(|$)/);
      }),
    );
  });
});

describe("nashville", () => {
  it("numbers a I-vi-IV-V and a minor song from its relative major", () => {
    const g: Key = { tonic_pc: 7, mode: "major" };
    const num = (root: number, quality: Chord["quality"], key = g) => {
      const n = nashville({ root, quality }, key);
      return n.degree + n.suffix;
    };
    expect([num(7, "maj"), num(4, "min"), num(0, "maj"), num(2, "7")]).toEqual(["1", "6m", "4", "57"]);
    expect(num(9, "min", { tonic_pc: 9, mode: "minor" })).toBe("6m");
    expect(num(5, "maj", g)).toBe("b7");
  });
});

describe("simplify", () => {
  it("keeps the root, is idempotent, and level 2 yields only triads", () => {
    const triads = new Set(["maj", "min", "dim", "aug"]);
    fc.assert(
      fc.property(chordArb, fc.constantFrom(0, 1, 2), (c, level) => {
        const s = simplify(c, level as 0 | 1 | 2);
        expect(s.root).toBe(c.root);
        expect(simplify(s, level as 0 | 1 | 2)).toEqual(s);
        if (level === 2) expect(triads.has(s.quality)).toBe(true);
      }),
    );
    expect(simplify({ root: 0, quality: "7" }, 1).quality).toBe("7");
    expect(simplify({ root: 0, quality: "maj7" }, 1).quality).toBe("maj");
  });
});

describe("capo suggestion", () => {
  const song = (names: [number, Chord["quality"]][]) => names.map(([root, quality]) => ({ chord: { root, quality }, ms: 2000 }));

  it("finds capo 1 for a song in Ab (G shapes) and no capo for a song in G", () => {
    const inAb = song([[8, "maj"], [5, "min"], [1, "maj"], [3, "maj"]]); // Ab Fm Db Eb
    const best = suggestCapo(inAb, { tonic_pc: 8, mode: "major" })[0];
    expect(best.capo).toBe(1);
    expect(best.shapes).toEqual(expect.arrayContaining(["G", "Em", "C", "D"]));
    expect(keyName(best.shapeKey!)).toBe("G");

    const inG = song([[7, "maj"], [4, "min"], [0, "maj"], [2, "maj"]]);
    expect(suggestCapo(inG, { tonic_pc: 7, mode: "major" })[0].capo).toBe(0);
  });

  it("weights by duration, not by count", () => {
    const chords = [
      { chord: { root: 10, quality: "maj" } as Chord, ms: 60_000 }, // Bb for a minute
      { chord: { root: 7, quality: "maj" } as Chord, ms: 500 },
      { chord: { root: 0, quality: "maj" } as Chord, ms: 500 },
    ];
    expect(suggestCapo(chords, null)[0].capo).toBe(1); // Bb becomes an A shape
  });

  it("returns nothing for an empty song", () => {
    expect(suggestCapo([], null)).toEqual([]);
  });
});
