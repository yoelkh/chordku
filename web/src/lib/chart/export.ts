// Text exports. ChordPro uses the grid environment (`{start_of_grid}`), which is exactly a
// chart of chords over bars without lyrics; the plain-text export is the same grid without
// directives. PDF is the browser's print dialog over the on-screen chart (see app.css).

import { buildBars, groupBySection, lines, type Bar } from "./layout";
import type { Chart } from "./model";
import { keySummary, labelAt, type ViewPrefs } from "./view";

export interface SongMeta {
  title: string;
  artist: string;
}

function barCells(chart: Chart, bar: Bar, prefs: ViewPrefs, width: number): string {
  // One cell per beat: the chord where it starts, "." elsewhere.
  return bar.beats
    .map((b) => (b.segment >= 0 ? labelAt(chart, b.segment, prefs).text.replace("N.C.", "n.c.") : ".").padEnd(width))
    .join(" ")
    .trimEnd();
}

function gridLines(chart: Chart, bars: Bar[], prefs: ViewPrefs): string[] {
  const width = Math.max(2, ...bars.flatMap((bar) => bar.beats.map((b) => (b.segment >= 0 ? labelAt(chart, b.segment, prefs).text.length : 1))));
  return lines(bars).map((row) => "| " + row.map((bar) => barCells(chart, bar, prefs, width)).join(" | ") + " |");
}

function timeList(chart: Chart, prefs: ViewPrefs): string[] {
  const clock = (ms: number) => `${Math.floor(ms / 60000)}:${String(Math.floor(ms / 1000) % 60).padStart(2, "0")}`;
  return chart.segments.map((s, i) => `${clock(s.t0_ms).padStart(5)}  ${labelAt(chart, i, prefs).text}`);
}

export function toChordPro(chart: Chart, prefs: ViewPrefs, meta: SongMeta): string {
  const out: string[] = [`{title: ${meta.title}}`];
  if (meta.artist) out.push(`{artist: ${meta.artist}}`);
  const key = keySummary(chart, prefs);
  if (key) out.push(`{key: ${key.shown}}`);
  if (prefs.capo > 0) out.push(`{capo: ${prefs.capo}}`);
  if (chart.bpm) out.push(`{tempo: ${Math.round(chart.bpm)}}`);
  out.push(`{time: ${chart.meter}/4}`, "");

  const bars = buildBars(chart);
  if (!bars.length) {
    out.push("{comment: Tanpa ketukan terdeteksi; daftar chord menurut waktu}", ...timeList(chart, prefs));
    return out.join("\n") + "\n";
  }
  for (const block of groupBySection(chart, bars)) {
    out.push(`{start_of_grid label="${block.section.label.replace(/"/g, "'")}" shape="${lines(block.bars)[0].length}x${chart.meter}"}`);
    out.push(...gridLines(chart, block.bars, prefs));
    out.push("{end_of_grid}", "");
  }
  return out.join("\n");
}

export function toText(chart: Chart, prefs: ViewPrefs, meta: SongMeta): string {
  const head = [meta.artist ? `${meta.title} - ${meta.artist}` : meta.title];
  const key = keySummary(chart, prefs);
  const facts = [
    key ? `Kunci ${key.shown}${prefs.capo > 0 ? ` (capo ${prefs.capo}, terdengar ${key.sounding})` : ""}` : "",
    chart.bpm ? `${Math.round(chart.bpm)} BPM` : "",
    `${chart.meter}/4`,
  ].filter(Boolean);
  head.push(facts.join("  |  "), "");

  const bars = buildBars(chart);
  if (!bars.length) return [...head, ...timeList(chart, prefs)].join("\n") + "\n";
  const body: string[] = [];
  for (const block of groupBySection(chart, bars)) {
    body.push(`[${block.section.label}]`, ...gridLines(chart, block.bars, prefs), "");
  }
  return [...head, ...body].join("\n");
}

export function download(filename: string, text: string, mime = "text/plain") {
  const url = URL.createObjectURL(new Blob([text], { type: `${mime};charset=utf-8` }));
  const a = document.createElement("a");
  a.href = url;
  a.download = filename.replace(/[\\/:*?"<>|]+/g, "_");
  a.click();
  setTimeout(() => URL.revokeObjectURL(url), 1000);
}
