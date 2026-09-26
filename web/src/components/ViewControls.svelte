<script lang="ts">
  // The view half of the song toolbar: transpose, capo (with suggestions), detail level,
  // notation, diagram instrument, floating diagrams, auto-scroll. Shared by the app's song
  // page and the standalone setlist viewer; page-specific buttons come in through `children`.
  import type { Snippet } from "svelte";
  import type { ViewPrefs } from "../lib/chart/view";
  import { keyName, wrapTranspose, type CapoOption } from "../lib/theory";

  let {
    prefs = $bindable(),
    capoOptions = [],
    floating,
    follow,
    showPitchFollow = true,
    onfloating,
    onfollow,
    children,
  }: {
    prefs: ViewPrefs;
    capoOptions?: CapoOption[];
    floating: boolean;
    follow: boolean;
    showPitchFollow?: boolean;
    onfloating: (on: boolean) => void;
    onfollow: (on: boolean) => void;
    children?: Snippet;
  } = $props();

  let showCapo = $state(false);
</script>

<div class="toolbar card no-print">
  <div class="group" title="Pindah nada dasar. Pintasan: [ dan ]">
    <span class="muted small">Transpose</span>
    <button class="icon" onclick={() => (prefs.transpose = wrapTranspose(prefs.transpose - 1))} aria-label="Turun setengah nada">−</button>
    <strong class="value mono">{prefs.transpose > 0 ? "+" : ""}{prefs.transpose}</strong>
    <button class="icon" onclick={() => (prefs.transpose = wrapTranspose(prefs.transpose + 1))} aria-label="Naik setengah nada">+</button>
    {#if showPitchFollow}
      <label class="row small check" title="Audio ikut berpindah nada"><input type="checkbox" bind:checked={prefs.pitch_follow} /> Audio ikut</label>
    {/if}
  </div>

  <div class="group capo">
    <span class="muted small">Capo</span>
    <button class="icon" onclick={() => (prefs.capo = Math.max(0, prefs.capo - 1))} aria-label="Kurangi capo">−</button>
    <strong class="value mono">{prefs.capo}</strong>
    <button class="icon" onclick={() => (prefs.capo = Math.min(11, prefs.capo + 1))} aria-label="Tambah capo">+</button>
    {#if capoOptions.length}
      <button class="small" class:on={showCapo} onclick={() => (showCapo = !showCapo)}>Saran</button>
    {/if}
    {#if showCapo}
      <div class="card pop">
        <div class="muted small">Posisi capo dengan bentuk chord termudah</div>
        {#each capoOptions as o}
          <button class="option" class:on={o.capo === prefs.capo} onclick={() => ((prefs.capo = o.capo), (showCapo = false))}>
            <strong>{o.capo === 0 ? "Tanpa capo" : `Capo ${o.capo}`}</strong>
            {#if o.shapeKey}<span class="pill">main di {keyName(o.shapeKey)}</span>{/if}
            <span class="muted small shapes">{o.shapes.join("  ")}</span>
          </button>
        {/each}
      </div>
    {/if}
  </div>

  <div class="group">
    <select bind:value={prefs.simplify} aria-label="Tingkat detail chord">
      <option value={0}>Chord lengkap</option>
      <option value={1}>Sederhana</option>
      <option value={2}>Dasar saja</option>
    </select>
    <select bind:value={prefs.notation} aria-label="Notasi">
      <option value="names">Nama chord</option>
      <option value="nashville">Angka (Nashville)</option>
    </select>
    <select bind:value={prefs.instrument} aria-label="Instrumen diagram">
      <option value="guitar">Gitar</option>
      <option value="ukulele">Ukulele</option>
      <option value="piano">Piano</option>
    </select>
  </div>

  <div class="group end">
    <button class:on={floating} onclick={() => onfloating(!floating)} aria-pressed={floating} title="Diagram chord yang tetap terlihat saat chart digulir">Diagram mengambang</button>
    <button class:on={follow} onclick={() => onfollow(!follow)} aria-pressed={follow} title="Gulir otomatis mengikuti lagu">Ikuti</button>
    {@render children?.()}
  </div>
</div>

<style>
  .toolbar {
    display: flex;
    flex-wrap: wrap;
    gap: 0.5rem 1.1rem;
    padding: 0.6rem 0.8rem;
    align-items: center;
    position: relative;
    z-index: 10;
  }
  .group {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 0.35rem;
    position: relative;
    min-width: 0;
  }
  .group select {
    flex: 1 1 7.5rem;
    min-width: 0;
    max-width: 100%;
  }
  .group.end {
    margin-left: auto;
    flex-wrap: wrap;
  }
  .value {
    min-width: 1.9rem;
    text-align: center;
    font-size: 1.05rem;
  }
  .check {
    gap: 0.3rem;
    margin-left: 0.3rem;
  }
  .pop {
    position: absolute;
    top: calc(100% + 0.4rem);
    left: 0;
    z-index: 40;
    width: max-content;
    min-width: min(17rem, calc(100vw - 2rem));
    max-width: calc(100vw - 2rem);
    max-height: 70vh;
    overflow: auto;
    padding: 0.5rem;
    display: grid;
    gap: 0.25rem;
    box-shadow: 0 14px 40px rgb(0 0 0 / 0.5);
  }
  .pop > .muted {
    padding: 0.35rem 0.4rem 0.1rem;
  }
  .option {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    flex-wrap: wrap;
    text-align: left;
    background: transparent;
    border-color: transparent;
    justify-content: space-between;
  }
  .option:hover {
    background: var(--bg-3);
  }
  .shapes {
    flex-basis: 100%;
  }
  @media (max-width: 640px) {
    .group.end {
      margin-left: 0;
    }
  }
</style>
