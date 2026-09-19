<script lang="ts">
  // Two octaves of keys with the chord tones lit, root position from the root upwards.
  import { chordTones, type Chord } from "../lib/theory";

  let { chord, name, size = 150 }: { chord: Chord; name: string; size?: number } = $props();

  const WHITE = [0, 2, 4, 5, 7, 9, 11];
  const BLACK: [pc: number, afterWhite: number][] = [[1, 0], [3, 1], [6, 3], [8, 4], [10, 5]];
  const W = 10;

  // Absolute keys (0..23) to press: root in the lower octave, other tones stacked above it.
  const pressed = $derived.by(() => {
    const tones = chordTones(chord);
    const root = tones[0];
    return new Set(tones.map((pc) => (pc >= root ? pc : pc + 12)));
  });
</script>

<figure class="piano" style="width:{size}px">
  <figcaption>{name}</figcaption>
  <svg viewBox="0 0 {W * 14} 44" role="img" aria-label="Diagram piano {name}">
    {#each [0, 1] as octave}
      {#each WHITE as pc, i}
        {@const on = pressed.has(pc + 12 * octave)}
        <rect x={(octave * 7 + i) * W} y="0" width={W} height="44" rx="1.5" fill={on ? "var(--accent)" : "#f4f4f0"} stroke="#555" stroke-width=".6" />
        {#if on && pc === chord.root}<circle cx={(octave * 7 + i) * W + W / 2} cy="37" r="2" fill="var(--accent-ink)" />{/if}
      {/each}
    {/each}
    {#each [0, 1] as octave}
      {#each BLACK as [pc, after]}
        {@const on = pressed.has(pc + 12 * octave)}
        <rect x={(octave * 7 + after + 1) * W - 3.2} y="0" width="6.4" height="27" rx="1" fill={on ? "var(--accent)" : "#1c1c1c"} stroke="#000" stroke-width=".5" />
        {#if on && pc === chord.root}<circle cx={(octave * 7 + after + 1) * W} cy="22" r="1.7" fill="var(--accent-ink)" />{/if}
      {/each}
    {/each}
  </svg>
</figure>

<style>
  .piano {
    margin: 0;
    display: grid;
    justify-items: center;
    gap: 0.25rem;
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
</style>
