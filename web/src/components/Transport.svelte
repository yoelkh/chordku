<script lang="ts">
  import { clock } from "../lib/i18n/id";

  let {
    playing,
    ready,
    loading,
    positionMs,
    durationMs,
    rate,
    loop,
    sections = [],
    ontoggle,
    onseek,
    onrate,
    onloop,
  }: {
    playing: boolean;
    ready: boolean;
    loading: number | null;
    positionMs: number;
    durationMs: number;
    rate: number;
    loop: { a: number; b: number } | null;
    sections?: { t0_ms: number; t1_ms: number }[];
    ontoggle: () => void;
    onseek: (ms: number) => void;
    onrate: (rate: number) => void;
    onloop: (loop: { a: number; b: number } | null) => void;
  } = $props();

  const RATES = [0.5, 0.6, 0.7, 0.8, 0.9, 1, 1.1, 1.25];
  let loopStart = $state<number | null>(null); // seconds, while the user is marking A..B

  function markLoop() {
    if (loop) {
      onloop(null);
      loopStart = null;
    } else if (loopStart === null) {
      loopStart = positionMs / 1000;
    } else {
      const a = Math.min(loopStart, positionMs / 1000);
      const b = Math.max(loopStart, positionMs / 1000);
      loopStart = null;
      if (b - a > 0.5) onloop({ a, b });
    }
  }

  const loopLabel = $derived(loop ? "Hapus loop" : loopStart === null ? "Tandai A" : "Tandai B");
  const pct = (ms: number) => (durationMs > 0 ? (100 * ms) / durationMs : 0);
</script>

<div class="transport card no-print">
  <button class="primary play" onclick={ontoggle} disabled={!ready} aria-label={playing ? "Jeda" : "Putar"}>
    {#if loading !== null}
      <span class="small">{Math.round(loading * 100)}%</span>
    {:else if playing}
      <svg viewBox="0 0 24 24" width="20" height="20"><path fill="currentColor" d="M6 5h4v14H6zm8 0h4v14h-4z" /></svg>
    {:else}
      <svg viewBox="0 0 24 24" width="20" height="20"><path fill="currentColor" d="M7 4.5v15l13-7.5z" /></svg>
    {/if}
  </button>

  <span class="mono small time">{clock(positionMs)}</span>
  <div class="seek grow">
    <div class="marks" aria-hidden="true">
      {#each sections as s}<i style="left:{pct(s.t0_ms)}%"></i>{/each}
      {#if loop}<b style="left:{pct(loop.a * 1000)}%; width:{pct((loop.b - loop.a) * 1000)}%"></b>{/if}
    </div>
    <input type="range" min="0" max={Math.max(1, durationMs)} step="100" value={positionMs} oninput={(e) => onseek(+e.currentTarget.value)} aria-label="Posisi lagu" disabled={!ready} />
  </div>
  <span class="mono small time muted">{clock(durationMs)}</span>

  <label class="rate row small">
    <span class="muted">Tempo</span>
    <select value={rate} onchange={(e) => onrate(+e.currentTarget.value)} aria-label="Tempo pemutaran">
      {#each RATES as r}<option value={r}>{Math.round(r * 100)}%</option>{/each}
    </select>
  </label>
  <button class="small" class:on={!!loop || loopStart !== null} onclick={markLoop} disabled={!ready}>{loopLabel}</button>
</div>

<style>
  .transport {
    position: fixed;
    /* Inset + auto margins instead of 50% / translate: never wider than the viewport. */
    left: 0.6rem;
    right: 0.6rem;
    margin: 0 auto;
    max-width: 1150px;
    bottom: max(0.7rem, env(safe-area-inset-bottom));
    z-index: 30;
    display: flex;
    align-items: center;
    gap: 0.6rem;
    padding: 0.55rem 0.7rem;
    box-shadow: 0 10px 40px rgb(0 0 0 / 0.45);
  }
  .play {
    width: 2.9rem;
    height: 2.9rem;
    border-radius: 50%;
    display: grid;
    place-items: center;
    padding: 0;
    flex: none;
  }
  .time {
    min-width: 2.6rem;
    text-align: center;
  }
  .seek {
    position: relative;
    display: grid;
  }
  .seek input {
    width: 100%;
    margin: 0;
  }
  .marks {
    position: absolute;
    inset: 0;
    pointer-events: none;
  }
  .marks i {
    position: absolute;
    top: 15%;
    bottom: 15%;
    width: 1px;
    background: var(--muted);
    opacity: 0.5;
  }
  .marks b {
    position: absolute;
    top: 30%;
    bottom: 30%;
    background: color-mix(in srgb, var(--accent) 40%, transparent);
    border-radius: 3px;
  }
  .rate select {
    padding: 0.25em 0.4em;
  }
  @media (max-width: 640px) {
    .transport {
      flex-wrap: wrap;
      row-gap: 0.35rem;
    }
    .seek {
      order: 5;
      flex-basis: 100%;
    }
  }
</style>
