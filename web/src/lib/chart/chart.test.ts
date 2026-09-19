import fc from "fast-check";
import { describe, expect, it } from "vitest";
import { toChordPro, toText } from "./export";
import { buildBars, groupBySection } from "./layout";
import { applyEdits, keyAt, segmentAt, type Analysis, type Edit } from "./model";
import { DEFAULT_PREFS, labelAt, labelFor, normalizePrefs } from "./view";

const C = 1, Am = 9 * 14, F = 5 * 14 + 1, G7 = 7 * 14 + 9, N = 169;

/** 16 bars of C | Am | F | G7 at 120 BPM (500 ms beats, 2 s bars), starting at 0. */
function analysis(): Analysis {
  const beats = Array.from({ length: 64 }, (_, i) => i * 500);
  const classes = [C, Am, F, G7];
  return {
    pipeline_ver: 1,
    duration_ms: 32_000,
    tuning_cents: 0,
    bpm: 120,
    meter: 4,
    beats_ms: beats,
    downbeat_idx: beats.map((_, i) => i).filter((i) => i % 4 === 0),
    key: { tonic_pc: 0, mode: "major", confidence: 0.8 },
    key_segments: [{ t0_ms: 0, tonic_pc: 0, mode: "major" }],
    segments: Array.from({ length: 16 }, (_, b) => ({ t0_ms: b * 2000, t1_ms: (b + 1) * 2000, class: classes[b % 4], conf: 0.9, alts: [] })),
    sections: [
      { t0_ms: 0, t1_ms: 16_000, label: "A" },
      { t0_ms: 16_000, t1_ms: 32_000, label: "B" },
    ],
  };
}

const edit = (id: number, op: Edit["op"], t0: number, t1: number, payload: Record<string, unknown> = {}): Edit => ({ id, op, t0_ms: t0, t1_ms: t1, payload });

describe("applyEdits", () => {
  it("without edits reproduces the analysis", () => {
    const chart = applyEdits(analysis(), []);
    expect(chart.segments).toHaveLength(16);
    expect(chart.key).toEqual({ tonic_pc: 0, mode: "major" });
  });

  it("set_chord splits what it overlaps and marks the result as edited", () => {
    const chart = applyEdits(analysis(), [edit(1, "set_chord", 1000, 3000, { class: F })]);
    expect(chart.segments.slice(0, 3).map((s) => [s.t0_ms, s.t1_ms, s.class])).toEqual([
      [0, 1000, C],
      [1000, 3000, F],
      [3000, 4000, Am],
    ]);
    expect(chart.segments[1].edited).toBe(true);
  });

  it("later edits win and equal neighbours merge", () => {
    const chart = applyEdits(analysis(), [
      edit(2, "set_chord", 0, 2000, { class: Am }),
      edit(1, "set_chord", 0, 2000, { class: F }),
      edit(3, "set_chord", 2000, 4000, { class: Am }),
    ]);
    expect(chart.segments[0]).toMatchObject({ t0_ms: 0, t1_ms: 4000, class: Am });
  });

  it("segments always tile the song exactly, whatever is edited", () => {
    const editArb = fc.record({
      t0: fc.integer({ min: 0, max: 31_000 }),
      len: fc.integer({ min: 1, max: 9000 }),
      cls: fc.integer({ min: 0, max: 169 }),
      del: fc.boolean(),
    });
    fc.assert(
      fc.property(fc.array(editArb, { maxLength: 12 }), (raw) => {
        const edits = raw.map((r, i) => edit(i + 1, r.del ? "delete" : "set_chord", r.t0, Math.min(32_000, r.t0 + r.len), { class: r.cls }));
        const { segments } = applyEdits(analysis(), edits);
        expect(segments[0].t0_ms).toBe(0);
        expect(segments[segments.length - 1].t1_ms).toBe(32_000);
        for (let i = 1; i < segments.length; i++) {
          expect(segments[i].t0_ms).toBe(segments[i - 1].t1_ms);
          expect(segments[i].t1_ms).toBeGreaterThan(segments[i].t0_ms);
        }
      }),
    );
  });

  it("overrides key, meter, downbeat and tempo level", () => {
    const chart = applyEdits(analysis(), [
      edit(1, "key_override", 0, 0, { tonic_pc: 9, mode: "minor" }),
      edit(2, "meter_override", 0, 0, { meter: 3 }),
      edit(3, "downbeat_shift", 0, 0, { shift: 1 }),
    ]);
    expect(chart.key).toEqual({ tonic_pc: 9, mode: "minor" });
    expect(chart.meter).toBe(3);
    expect(chart.downbeat_idx.slice(0, 3)).toEqual([1, 4, 7]);

    const doubled = applyEdits(analysis(), [edit(1, "tempo_scale", 0, 0, { factor: 2 })]);
    expect(doubled.bpm).toBe(240);
    expect(doubled.beats_ms.slice(0, 3)).toEqual([0, 250, 500]);
    const halved = applyEdits(analysis(), [edit(1, "tempo_scale", 0, 0, { factor: 0.5 })]);
    expect(halved.beats_ms.slice(0, 3)).toEqual([0, 1000, 2000]);
  });

  it("renames the section at the given time", () => {
    const chart = applyEdits(analysis(), [edit(1, "section_rename", 17_000, 17_000, { label: "Reff" })]);
    expect(chart.sections.map((s) => s.label)).toEqual(["A", "Reff"]);
  });
});

describe("lookup", () => {
  it("segmentAt and keyAt", () => {
    const a = analysis();
    a.key_segments.push({ t0_ms: 16_000, tonic_pc: 2, mode: "major" });
    const chart = applyEdits(a, []);
    expect(segmentAt(chart.segments, 0)).toBe(0);
    expect(segmentAt(chart.segments, 1999)).toBe(0);
    expect(segmentAt(chart.segments, 2000)).toBe(1);
    expect(segmentAt(chart.segments, 99_999)).toBe(-1);
    expect(keyAt(chart, 1000)?.tonic_pc).toBe(0);
    expect(keyAt(chart, 20_000)?.tonic_pc).toBe(2);
  });
});

describe("layout", () => {
  it("prints each chord once, on the beat where it starts", () => {
    const chart = applyEdits(analysis(), []);
    const bars = buildBars(chart);
    expect(bars).toHaveLength(16);
    expect(bars[1].beats.map((b) => b.segment)).toEqual([1, -1, -1, -1]);
    expect(bars[1].beats.every((b) => b.sounding === 1)).toBe(true);
    expect(groupBySection(chart, bars).map((b) => b.bars.length)).toEqual([8, 8]);
  });

  it("has no bars without a beat grid", () => {
    const a = analysis();
    a.beats_ms = [];
    a.downbeat_idx = [];
    expect(buildBars(applyEdits(a, []))).toEqual([]);
  });
});

describe("view", () => {
  const key = { tonic_pc: 0, mode: "major" } as const;

  it("transpose moves the sound, capo only the shapes", () => {
    const prefs = { ...DEFAULT_PREFS, transpose: 2, capo: 2 };
    const label = labelFor(G7, key, prefs);
    expect(label.sounding).toEqual({ root: 9, quality: "7" }); // A7 sounds
    expect(label.shape).toEqual({ root: 7, quality: "7" }); // G7 is fingered
    expect(label.text).toBe("G7");
  });

  it("spells in the displayed key", () => {
    // C major up 3 = Eb major: the IV chord must read Ab.
    expect(labelFor(F, key, { ...DEFAULT_PREFS, transpose: 3 }).text).toBe("Ab");
    // C major up 4 = E major: the vi chord must read C#m.
    expect(labelFor(Am, key, { ...DEFAULT_PREFS, transpose: 4 }).text).toBe("C#m");
  });

  it("Nashville, no-chord and defaults", () => {
    expect(labelFor(Am, key, { ...DEFAULT_PREFS, notation: "nashville", transpose: 5 }).text).toBe("6m");
    expect(labelFor(N, key, DEFAULT_PREFS).text).toBe("N.C.");
    expect(labelFor(168, key, DEFAULT_PREFS).text).toBe("?");
    expect(normalizePrefs({ transpose: 99, capo: -3, notation: "x", instrument: "piano" })).toMatchObject({
      transpose: 6, capo: 0, notation: "names", instrument: "piano", pitch_follow: true,
    });
    expect(labelAt(applyEdits(analysis(), []), 2, DEFAULT_PREFS).text).toBe("F");
  });
});

describe("exports", () => {
  const meta = { title: "Lagu Uji", artist: "Band" };

  it("ChordPro uses the grid environment with one cell per beat", () => {
    const text = toChordPro(applyEdits(analysis(), []), { ...DEFAULT_PREFS, capo: 2 }, meta);
    expect(text).toContain("{title: Lagu Uji}");
    expect(text).toContain("{key: Bb}");
    expect(text).toContain("{capo: 2}");
    expect(text).toContain('{start_of_grid label="A" shape="4x4"}');
    expect(text).toContain("| Bb .  .  . | Gm .  .  . | Eb .  .  . | F7 .  .  . |");
    expect(text.match(/\{end_of_grid\}/g)).toHaveLength(2);
  });

  it("plain text names the sounding key when a capo is set", () => {
    const text = toText(applyEdits(analysis(), []), { ...DEFAULT_PREFS, capo: 2 }, meta);
    expect(text.split("\n")[0]).toBe("Lagu Uji - Band");
    expect(text).toContain("Kunci Bb (capo 2, terdengar C)");
    expect(text).toContain("[B]");
  });

  it("falls back to a timed list without beats", () => {
    const a = analysis();
    a.beats_ms = [];
    a.downbeat_idx = [];
    expect(toText(applyEdits(a, []), DEFAULT_PREFS, meta)).toContain(" 0:02  Am");
  });
});
