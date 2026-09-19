<script lang="ts">
  import type { Segment } from "../lib/chart/model";
  import { labelFor, type ViewPrefs } from "../lib/chart/view";
  import { clock } from "../lib/i18n/id";
  import { CLASS_NO_CHORD, QUALITIES, chordToClass, chordTones, classToChord, mod12, noteName, prefersFlats, qualitySuffix, type Key, type Quality } from "../lib/theory";

  let {
    segment,
    range,
    keyNow,
    prefs,
    onsave,
    onclose,
    onaudition,
  }: {
    segment: Segment;
    /** What will be overwritten. */
    range: { t0_ms: number; t1_ms: number };
    keyNow: Key | null;
    prefs: ViewPrefs;
    onsave: (cls: number) => void;
    onclose: () => void;
    onaudition: (pcs: number[]) => void;
  } = $props();

  // The picker works in DISPLAYED pitch (what the user reads); convert back on save.
  const offset = $derived(prefs.transpose - prefs.capo);
  const flats = $derived(prefersFlats(keyNow ? { tonic_pc: mod12(keyNow.tonic_pc + offset), mode: keyNow.mode } : null));
  // The dialog is created per edit, so seeding the picker from the props once is intended.
  const seed = () => {
    const c = classToChord(segment.class);
    return { root: c ? mod12(c.root + prefs.transpose - prefs.capo) : 0, quality: (c?.quality ?? "maj") as Quality };
  };
  let root = $state(seed().root);
  let quality = $state<Quality>(seed().quality);

  const candidates = $derived([{ class: segment.class, p: segment.conf }, ...segment.alts].filter((c) => c.class < 168));
  const pickedClass = $derived(chordToClass({ root: mod12(root - offset), quality }));
  const label = (cls: number) => labelFor(cls, keyNow, { ...prefs, simplify: 0, notation: "names" }).text;

  function audition(cls: number) {
    const chord = classToChord(cls);
    if (chord) onaudition(chordTones({ root: mod12(chord.root + prefs.transpose), quality: chord.quality }));
  }

  function onkey(e: KeyboardEvent) {
    if (e.key === "Escape") onclose();
  }
</script>

<svelte:window onkeydown={onkey} />

<div class="backdrop no-print" role="presentation" onclick={(e) => e.target === e.currentTarget && onclose()}>
  <div class="card dialog" role="dialog" aria-modal="true" aria-label="Koreksi chord">
    <div class="row">
      <h3 class="grow">Koreksi chord</h3>
      <span class="muted small mono">{clock(range.t0_ms)} - {clock(range.t1_ms)}</span>
    </div>

    {#if candidates.length}
      <div>
        <div class="muted small">Kandidat dari pengenal (ketuk untuk memilih)</div>
        <div class="row wrap cands">
          {#each candidates as c, i}
            <button class:on={i === 0} onclick={() => onsave(c.class)} oncontextmenu={(e) => (e.preventDefault(), audition(c.class))}>
              <strong>{label(c.class)}</strong>
              <span class="muted small">{Math.round(c.p * 100)}%</span>
            </button>
          {/each}
        </div>
      </div>
    {/if}

    <div>
      <div class="muted small">Atau pilih sendiri</div>
      <div class="roots">
        {#each Array(12) as _, pc}
          <button class="icon" class:on={root === pc} onclick={() => (root = pc)}>{noteName(pc, flats)}</button>
        {/each}
      </div>
      <div class="quals">
        {#each QUALITIES as q}
          <button class="small" class:on={quality === q} onclick={() => (quality = q)}>{noteName(root, flats)}{qualitySuffix(q)}</button>
        {/each}
      </div>
    </div>

    <div class="row wrap actions">
      <button onclick={() => audition(pickedClass)}>Dengarkan</button>
      <button class="danger" onclick={() => onsave(CLASS_NO_CHORD)}>Tanpa chord (N.C.)</button>
      <span class="grow"></span>
      <button class="ghost" onclick={onclose}>Batal</button>
      <button class="primary" onclick={() => onsave(pickedClass)}>Pakai {noteName(root, flats)}{qualitySuffix(quality)}</button>
    </div>
  </div>
</div>

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    z-index: 50;
    background: rgb(0 0 0 / 0.55);
    display: grid;
    place-items: center;
    padding: 1rem;
  }
  .dialog {
    width: min(34rem, 100%);
    max-height: calc(100vh - 2rem);
    overflow: auto;
    padding: 1rem;
    display: grid;
    gap: 0.9rem;
  }
  .cands {
    margin-top: 0.3rem;
  }
  .cands button {
    display: grid;
    justify-items: center;
    min-width: 4.4rem;
  }
  .roots {
    display: grid;
    grid-template-columns: repeat(6, 1fr);
    gap: 0.3rem;
    margin: 0.3rem 0 0.5rem;
  }
  .quals {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(5.2rem, 1fr));
    gap: 0.3rem;
  }
  .actions {
    gap: 0.4rem;
  }
</style>
