import { describe, expect, it } from "vitest";
import { drumBars, drumTabText, stepAt } from "./drums";
import type { Bar } from "./layout";

// Two 4/4 bars at 120 BPM (500 ms beats).
const bars: Bar[] = [0, 1].map((i) => ({
  index: i,
  t0_ms: i * 2000,
  t1_ms: (i + 1) * 2000,
  beats: [0, 1, 2, 3].map((b) => ({ t_ms: i * 2000 + b * 500, segment: -1, sounding: -1 })),
}));

describe("drum tab quantisation", () => {
  it("places hits on 16th steps and keeps the strongest per step", () => {
    const tab = drumBars(bars, {
      version: 1,
      onsets: [
        { t_ms: 10, kick: 90, snare: 0, hat: 40 },
        { t_ms: 1000, kick: 0, snare: 100, hat: 0 },
        { t_ms: 1250, kick: 70, snare: 0, hat: 0 }, // the "and" of beat 3
        { t_ms: 1990, kick: 80, snare: 0, hat: 0 }, // rounds into bar 2, step 0
        { t_ms: 2005, kick: 60, snare: 0, hat: 0 },
      ],
    });
    expect(tab[0].kick[0]).toBe(90);
    expect(tab[0].hat[0]).toBe(40);
    expect(tab[0].snare[8]).toBe(100);
    expect(tab[0].kick[10]).toBe(70);
    expect(tab[1].kick[0]).toBe(80);
  });

  it("ignores weak hits and completes a steady hi-hat pattern", () => {
    const onsets = [];
    for (let s = 0; s < 8; s++) if (s !== 2 && s !== 5) onsets.push({ t_ms: s * 250, kick: 0, snare: 0, hat: 70 });
    onsets.push({ t_ms: 1000, kick: 10, snare: 100, hat: 0 }); // kick 10: noise, stays off
    const tab = drumBars(bars, { version: 1, onsets });
    expect(tab[0].kick[8]).toBe(0);
    expect(tab[0].hat.filter((v) => v > 0).length).toBe(8);
    expect(tab[0].hatInferred[4]).toBe(true);
    expect(tab[0].hatInferred[0]).toBe(false);
    expect(tab[1].hat.every((v) => v === 0)).toBe(true); // an empty bar gets nothing invented
  });

  it("finds the step under the playhead and prints a tab", () => {
    expect(stepAt(bars[0], 0)).toBe(0);
    expect(stepAt(bars[0], 1249)).toBe(9);
    expect(stepAt(bars[0], 1999)).toBe(15);
    expect(stepAt(bars[1], 1999)).toBe(-1);
    const text = drumTabText(drumBars(bars, { version: 1, onsets: [{ t_ms: 0, kick: 100, snare: 0, hat: 0 }] }));
    expect(text.split("\n")[2]).toBe("BD |X---------------|----------------|");
  });
});
