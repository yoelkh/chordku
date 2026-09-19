<script lang="ts">
  // Fretboard diagram for guitar (6 strings) and ukulele (4). Voicings come from
  // @tombatossals/chords-db (MIT), trimmed by scripts/gen-voicings.mjs.
  import voicings from "../lib/diagrams/voicings.json";
  import type { Chord } from "../lib/theory";

  type Voicing = [frets: number[], fingers: number[], baseFret: number, barres: number[]];

  let { chord, instrument, name, variant = 0, size = 92 }: { chord: Chord; instrument: "guitar" | "ukulele"; name: string; variant?: number; size?: number } = $props();

  const table = voicings as unknown as Record<string, Record<string, Voicing[]>>;
  const options = $derived(table[instrument][`${chord.root}:${chord.quality}`] ?? []);
  const voicing = $derived(options.length ? options[variant % options.length] : null);

  const FRETS = 4;
  const strings = $derived(instrument === "guitar" ? 6 : 4);
  // Geometry in a 100 x 120 box.
  const left = 14, right = 92, top = 26, bottom = 110;
  const sx = (s: number) => left + ((right - left) * s) / (strings - 1);
  const fy = (f: number) => top + ((bottom - top) * f) / FRETS;

  const barres = $derived.by(() => {
    if (!voicing) return [];
    const [frets, , , list] = voicing;
    return list.map((fret) => {
      const hit = frets.map((f, i) => (f === fret ? i : -1)).filter((i) => i >= 0);
      return { fret, from: Math.min(...hit), to: Math.max(...hit) };
    });
  });
</script>

<figure class="diagram" style="width:{size}px">
  <figcaption>{name}</figcaption>
  {#if voicing}
    {@const [frets, fingers, baseFret] = voicing}
    <svg viewBox="0 0 100 120" role="img" aria-label="Diagram {name} untuk {instrument === 'guitar' ? 'gitar' : 'ukulele'}">
      {#if baseFret === 1}
        <rect x={left - 1} y={top - 4} width={right - left + 2} height="4" rx="1" fill="currentColor" />
      {:else}
        <text x="1" y={fy(0.5) + 3} font-size="10" fill="currentColor">{baseFret}</text>
      {/if}
      {#each Array(FRETS + 1) as _, f}
        <line x1={left} x2={right} y1={fy(f)} y2={fy(f)} stroke="currentColor" stroke-width="1" opacity=".55" />
      {/each}
      {#each Array(strings) as _, s}
        <line x1={sx(s)} x2={sx(s)} y1={top} y2={bottom} stroke="currentColor" stroke-width="1" opacity=".55" />
      {/each}
      {#each barres as b}
        <rect x={sx(b.from) - 5} y={fy(b.fret - 0.5) - 5} width={sx(b.to) - sx(b.from) + 10} height="10" rx="5" fill="var(--accent)" />
      {/each}
      {#each frets as f, s}
        {#if f === -1}
          <text x={sx(s)} y={top - 9} text-anchor="middle" font-size="10" fill="currentColor" opacity=".7">×</text>
        {:else if f === 0}
          <circle cx={sx(s)} cy={top - 12} r="3.4" fill="none" stroke="currentColor" stroke-width="1.2" opacity=".8" />
        {:else}
          <circle cx={sx(s)} cy={fy(f - 0.5)} r="5.6" fill="var(--accent)" />
          {#if fingers[s] > 0}
            <text x={sx(s)} y={fy(f - 0.5) + 3.2} text-anchor="middle" font-size="8.5" font-weight="700" fill="var(--accent-ink)">{fingers[s]}</text>
          {/if}
        {/if}
      {/each}
    </svg>
  {:else}
    <div class="none muted small">tanpa diagram</div>
  {/if}
</figure>

<style>
  .diagram {
    margin: 0;
    display: grid;
    justify-items: center;
    gap: 0.1rem;
    color: var(--text);
  }
  figcaption {
    font-weight: 700;
    font-size: 0.95rem;
    white-space: nowrap;
  }
  svg {
    width: 100%;
    height: auto;
    display: block;
  }
  .none {
    aspect-ratio: 100 / 120;
    display: grid;
    place-items: center;
    width: 100%;
  }
</style>
