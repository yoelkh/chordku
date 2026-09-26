<script lang="ts">
  import { drumBars, stepAt, type DrumEvents } from "../lib/chart/drums";
  import { buildBars, groupBySection, lines } from "../lib/chart/layout";
  import type { Chart } from "../lib/chart/model";
  import { labelAt, type ViewPrefs } from "../lib/chart/view";
  import { clock } from "../lib/i18n/id";

  export interface CellTarget {
    /** Segment sounding at the tapped beat. */
    segment: number;
    /** Time of the tapped beat: where a new chord would start. */
    t_ms: number;
    /** True if the tapped cell is where the segment starts. */
    isStart: boolean;
  }

  let {
    chart,
    prefs,
    positionMs,
    editing = false,
    follow = true,
    loop = null,
    drums = null,
    onseek,
    onedit,
    onloop,
    onrename,
  }: {
    chart: Chart;
    prefs: ViewPrefs;
    positionMs: number;
    editing?: boolean;
    follow?: boolean;
    loop?: { a: number; b: number } | null;
    /** Drum hits to show as a tab under the chords; null hides the lanes. */
    drums?: DrumEvents | null;
    onseek: (ms: number) => void;
    onedit: (target: CellTarget) => void;
    onloop: (range: { a: number; b: number } | null) => void;
    onrename: (t_ms: number, current: string) => void;
  } = $props();

  const bars = $derived(buildBars(chart));
  const blocks = $derived(groupBySection(chart, bars));
  const labels = $derived(chart.segments.map((_, i) => labelAt(chart, i, prefs).text));

  // Active bar / beat from the playhead (binary search over bars, then beats).
  const active = $derived.by(() => {
    let lo = 0;
    let hi = bars.length - 1;
    while (lo <= hi) {
      const mid = (lo + hi) >> 1;
      if (positionMs < bars[mid].t0_ms) hi = mid - 1;
      else if (positionMs >= bars[mid].t1_ms) lo = mid + 1;
      else {
        const beats = bars[mid].beats;
        let beat = 0;
        for (let i = 0; i < beats.length; i++) if (beats[i].t_ms <= positionMs) beat = i;
        return { bar: mid, beat };
      }
    }
    return { bar: -1, beat: -1 };
  });

  const tab = $derived(drums ? drumBars(bars, drums) : null);
  const activeStep = $derived(tab && active.bar >= 0 ? stepAt(bars[active.bar], positionMs) : -1);
  const LANES = [["hat", "HH"], ["snare", "SD"], ["kick", "BD"]] as const;

  // Keep the playing line in view.
  let root: HTMLElement;
  let lastLine = -1;
  $effect(() => {
    const line = active.bar < 0 ? -1 : Math.floor(active.bar / 4);
    if (!follow || editing || line === lastLine) return;
    lastLine = line;
    root?.querySelector<HTMLElement>(`[data-bar="${active.bar}"]`)?.scrollIntoView({ block: "center", behavior: "smooth" });
  });

  const isLooping = (t0: number, t1: number) => !!loop && Math.abs(loop.a * 1000 - t0) < 30 && Math.abs(loop.b * 1000 - t1) < 30;

  function cell(barIndex: number, beatIndex: number) {
    const b = bars[barIndex].beats[beatIndex];
    if (editing) {
      if (b.sounding >= 0) onedit({ segment: b.sounding, t_ms: b.t_ms, isStart: b.segment >= 0 });
    } else {
      onseek(b.t_ms);
    }
  }

  const activeSegment = $derived.by(() => {
    const s = chart.segments;
    for (let i = 0; i < s.length; i++) if (positionMs >= s[i].t0_ms && positionMs < s[i].t1_ms) return i;
    return -1;
  });
</script>

<div class="chart" class:editing bind:this={root}>
  {#if bars.length}
    {#each blocks as block (block.section.t0_ms)}
      <section>
        <header class="row">
          <button class="ghost label" title="Ganti nama bagian" onclick={() => onrename(block.section.t0_ms + 1, block.section.label)}>{block.section.label}</button>
          <span class="muted small mono">{clock(block.section.t0_ms)}</span>
          <button
            class="ghost small no-print"
            class:on={isLooping(block.section.t0_ms, block.section.t1_ms)}
            onclick={() => onloop(isLooping(block.section.t0_ms, block.section.t1_ms) ? null : { a: block.section.t0_ms / 1000, b: block.section.t1_ms / 1000 })}
          >
            {isLooping(block.section.t0_ms, block.section.t1_ms) ? "Berhenti mengulang" : "Ulang bagian ini"}
          </button>
        </header>
        {#each lines(block.bars) as line}
          <div class="line" style="--bars:{line.length}">
            {#each line as bar (bar.index)}
              <div class="bar-cell" class:now={bar.index === active.bar} data-bar={bar.index} style="--beats:{bar.beats.length}">
                {#each bar.beats as beat, i}
                  <button
                    class="beat"
                    class:now={bar.index === active.bar && i === active.beat}
                    class:start={beat.segment >= 0}
                    class:edited={beat.segment >= 0 && chart.segments[beat.segment].edited}
                    class:unsure={beat.segment >= 0 && chart.segments[beat.segment].conf < 0.4}
                    onclick={() => cell(bar.index, i)}
                    aria-label={beat.segment >= 0 ? labels[beat.segment] : "ketukan"}
                  >
                    {#if beat.segment >= 0}{labels[beat.segment]}{:else}<span class="dot">·</span>{/if}
                  </button>
                {/each}
                {#if tab}
                  {@const d = tab[bar.index]}
                  <div class="drums" style="--steps:{d.steps}" aria-label="Tab drum bar {bar.index + 1}">
                    {#each LANES as [lane, name]}
                      {#each d[lane] as v, s}
                        <span
                          class="step {lane}"
                          class:hit={v > 0}
                          class:soft={v > 0 && v < 60}
                          class:inferred={lane === "hat" && d.hatInferred[s]}
                          class:now={bar.index === active.bar && s === activeStep}
                          class:beat={s % 4 === 0}
                          title={v > 0 ? `${name} ${v}` : ""}
                        ></span>
                      {/each}
                    {/each}
                  </div>
                {/if}
              </div>
            {/each}
          </div>
        {/each}
      </section>
    {/each}
  {:else}
    <p class="muted small no-print">Ketukan tidak terdeteksi (misalnya lagu tanpa ritme jelas), jadi chord ditampilkan menurut waktu.</p>
    <div class="flow">
      {#each chart.segments as s, i}
        <button class="chip" class:now={i === activeSegment} class:edited={s.edited} onclick={() => (editing ? onedit({ segment: i, t_ms: s.t0_ms, isStart: true }) : onseek(s.t0_ms))}>
          <span class="muted small mono">{clock(s.t0_ms)}</span>
          <strong>{labels[i]}</strong>
        </button>
      {/each}
    </div>
  {/if}
</div>

<style>
  .chart {
    display: grid;
    gap: 1.1rem;
  }
  section {
    display: grid;
    gap: 0.3rem;
    break-inside: avoid;
  }
  header {
    gap: 0.5rem;
  }
  .label {
    font-weight: 700;
    color: var(--accent);
    padding: 0.1em 0.5em;
    min-width: 2em;
  }
  .line {
    display: grid;
    grid-template-columns: repeat(4, minmax(0, 1fr));
    border-left: 2px solid var(--muted);
  }
  .bar-cell {
    display: grid;
    grid-template-columns: repeat(var(--beats), minmax(0, 1fr));
    border-right: 2px solid var(--muted);
    min-height: 2.9rem;
    background: var(--bg-2);
  }
  .bar-cell.now {
    background: color-mix(in srgb, var(--accent) 9%, var(--bg-2));
  }
  .beat {
    border: 0;
    border-radius: 0;
    background: transparent;
    padding: 0.2rem 0.15rem 0.2rem 0.4rem;
    text-align: left;
    font-weight: 700;
    font-size: clamp(0.85rem, 1.9vw, 1.12rem);
    white-space: nowrap;
    overflow: visible;
    position: relative;
    z-index: 0;
  }
  .beat.start {
    z-index: 1;
  }
  .beat.now {
    background: color-mix(in srgb, var(--accent) 30%, transparent);
    box-shadow: inset 0 -3px 0 var(--accent);
  }
  .beat.unsure {
    color: var(--muted);
  }
  .beat.edited {
    color: var(--good);
  }
  .dot {
    color: var(--line);
    font-weight: 400;
  }
  .drums {
    grid-column: 1 / -1;
    display: grid;
    grid-template-columns: repeat(var(--steps), minmax(0, 1fr));
    grid-auto-rows: 0.62rem;
    gap: 1px 0;
    padding: 0.1rem 0.15rem 0.25rem;
    border-top: 1px dashed var(--line);
  }
  .step {
    position: relative;
    border-left: 1px solid transparent;
  }
  .step.beat {
    border-left-color: var(--line);
  }
  .step.hit::after {
    content: "";
    position: absolute;
    inset: 1px 15%;
    border-radius: 3px;
  }
  .step.hat.hit::after {
    background: var(--muted);
  }
  .step.snare.hit::after {
    background: var(--good);
  }
  .step.kick.hit::after {
    background: var(--accent);
  }
  .step.soft::after {
    opacity: 0.5;
  }
  .step.inferred::after {
    opacity: 0.3;
  }
  .step.now {
    background: color-mix(in srgb, var(--accent) 22%, transparent);
  }
  .editing .beat:hover {
    outline: 1px dashed var(--accent);
    outline-offset: -2px;
  }
  .flow {
    display: flex;
    flex-wrap: wrap;
    gap: 0.4rem;
  }
  .chip {
    display: grid;
    justify-items: start;
    padding: 0.3rem 0.7rem;
    min-width: 4.2rem;
  }
  .chip.now {
    border-color: var(--accent);
    background: color-mix(in srgb, var(--accent) 18%, var(--bg-3));
  }
  .chip.edited strong {
    color: var(--good);
  }
  @media (max-width: 640px) {
    .line {
      grid-template-columns: repeat(2, minmax(0, 1fr));
      row-gap: 2px;
    }
  }
  @media print {
    .bar-cell,
    .bar-cell.now,
    .beat.now {
      background: none;
      box-shadow: none;
    }
    .beat {
      color: #000 !important;
      font-size: 12pt;
    }
    .step.hit::after {
      background: #000 !important;
      opacity: 1;
    }
    .step.soft::after,
    .step.inferred::after {
      opacity: 0.45;
    }
  }
</style>
