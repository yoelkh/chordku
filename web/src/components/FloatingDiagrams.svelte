<script lang="ts">
  // A chord cheat-sheet that floats over the chart: it stays put while the chart scrolls,
  // can be dragged anywhere, collapsed to a chip, and shows guitar, ukulele, piano or all
  // three for the chord sounding now, plus the next ones coming up.
  import type { Chord } from "../lib/theory";
  import ChordDiagram from "./ChordDiagram.svelte";
  import PianoDiagram from "./PianoDiagram.svelte";

  export interface DiagramItem {
    /** Printed name ("F#m7"), or "6m = F#m7" style when the chart shows numbers. */
    name: string;
    /** As fingered (after transpose, capo). */
    shape: Chord;
    /** As it sounds (piano). */
    sounding: Chord;
  }
  type Mode = "guitar" | "ukulele" | "piano" | "all";

  let { items, mode, onmode, onclose }: { items: DiagramItem[]; mode: Mode; onmode: (m: Mode) => void; onclose: () => void } = $props();

  const STORE = "chordku.floating";
  const saved = (() => {
    try {
      return JSON.parse(localStorage.getItem(STORE) ?? "{}") as { x?: number; y?: number; collapsed?: boolean };
    } catch {
      return {};
    }
  })();

  let panel = $state<HTMLElement>();
  // On a phone the panel starts as a one-line chip (current chord, next chord): a tap opens
  // the diagrams. The user's own choice, once made, wins.
  let collapsed = $state(saved.collapsed ?? matchMedia("(max-width: 640px)").matches);
  // null = default dock (bottom right, above the transport bar).
  let pos = $state<{ x: number; y: number } | null>(typeof saved.x === "number" && typeof saved.y === "number" ? { x: saved.x, y: saved.y } : null);
  let variants = $state<Record<string, number>>({});

  const persist = () => localStorage.setItem(STORE, JSON.stringify({ ...(pos ?? {}), collapsed }));

  function clamp(p: { x: number; y: number }) {
    // A minimised or hidden window reports a 0 x 0 viewport: leave the position alone then,
    // or the panel would be shoved into the corner for the rest of the session.
    if (innerWidth < 100 || innerHeight < 100) return p;
    const w = panel?.offsetWidth ?? 200;
    const h = panel?.offsetHeight ?? 100;
    // Never cover the transport bar at the bottom.
    const reserved = (document.querySelector<HTMLElement>(".transport")?.offsetHeight ?? 0) + 16;
    return { x: Math.max(4, Math.min(p.x, innerWidth - w - 4)), y: Math.max(4, Math.min(p.y, innerHeight - h - reserved)) };
  }

  // Phones get smaller diagrams: the panel must leave the chart readable.
  let compact = $state(matchMedia("(max-width: 640px)").matches);
  $effect(() => {
    const mq = matchMedia("(max-width: 640px)");
    const on = () => (compact = mq.matches);
    mq.addEventListener("change", on);
    return () => mq.removeEventListener("change", on);
  });
  const size = $derived(
    compact
      ? { fret: 84, piano: 150, allGuitar: 74, allUke: 60, allPiano: 116, nextFret: 50, nextPiano: 80 }
      : { fret: 118, piano: 190, allGuitar: 104, allUke: 84, allPiano: 150, nextFret: 66, nextPiano: 104 },
  );

  // Dragging by the header (mouse, pen and touch through pointer events).
  let drag: { dx: number; dy: number } | null = null;
  function down(e: PointerEvent) {
    if ((e.target as HTMLElement).closest("button") || !panel) return;
    const r = panel.getBoundingClientRect();
    drag = { dx: e.clientX - r.left, dy: e.clientY - r.top };
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
    e.preventDefault();
  }
  function move(e: PointerEvent) {
    if (drag) pos = clamp({ x: e.clientX - drag.dx, y: e.clientY - drag.dy });
  }
  function up() {
    if (drag) persist();
    drag = null;
  }

  // Keep the panel on screen when the window or its own size changes.
  $effect(() => {
    const fix = () => pos && (pos = clamp(pos));
    addEventListener("resize", fix);
    return () => removeEventListener("resize", fix);
  });
  $effect(() => {
    void mode, collapsed, items.length, compact;
    queueMicrotask(() => pos && (pos = clamp(pos)));
  });

  const current = $derived(items[0]);
  const upcoming = $derived(items.slice(1, compact ? 3 : 4));
  const key = (it: DiagramItem) => `${it.shape.root}:${it.shape.quality}`;
  const cycle = (it: DiagramItem) => (variants[key(it)] = (variants[key(it)] ?? 0) + 1);
  const MODES: [Mode, string][] = [["guitar", "Gitar"], ["ukulele", "Ukulele"], ["piano", "Piano"], ["all", "Semua"]];
</script>

<aside
  class="floating card no-print"
  class:collapsed
  class:docked={!pos}
  bind:this={panel}
  style={pos ? `left:${pos.x}px; top:${pos.y}px` : ""}
  aria-label="Diagram chord mengambang"
>
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <header onpointerdown={down} onpointermove={move} onpointerup={up} onpointercancel={up} title="Seret untuk memindahkan">
    <span class="grip" aria-hidden="true">⠿</span>
    {#if collapsed}
      <strong class="now">{current?.name ?? "N.C."}</strong>
      {#if upcoming[0]}<span class="muted small">lalu {upcoming[0].name}</span>{/if}
    {:else}
      <div class="tabs">
        {#each MODES as [m, label]}
          <button class="small" class:on={mode === m} onclick={() => onmode(m)}>{label}</button>
        {/each}
      </div>
    {/if}
    <span class="grow"></span>
    <button class="ghost icon small" onclick={() => ((collapsed = !collapsed), persist())} aria-label={collapsed ? "Buka diagram" : "Ciutkan diagram"}>{collapsed ? "▢" : "–"}</button>
    <button class="ghost icon small" onclick={onclose} aria-label="Tutup diagram mengambang">✕</button>
  </header>

  {#if !collapsed}
    <div class="body">
      {#if !current}
        <p class="muted small">Tidak ada chord saat ini.</p>
      {:else if mode === "all"}
        <div class="all">
          <button class="plain" onclick={() => cycle(current)} title="Ketuk untuk posisi lain">
            <ChordDiagram chord={current.shape} instrument="guitar" name={current.name} variant={variants[key(current)] ?? 0} size={size.allGuitar} />
          </button>
          <button class="plain" onclick={() => cycle(current)} title="Ketuk untuk posisi lain">
            <ChordDiagram chord={current.shape} instrument="ukulele" name="Ukulele" variant={variants[key(current)] ?? 0} size={size.allUke} />
          </button>
          <PianoDiagram chord={current.sounding} name="Piano" size={size.allPiano} />
        </div>
      {:else}
        <div class="now-next">
          <div class="current">
            {#if mode === "piano"}
              <PianoDiagram chord={current.sounding} name={current.name} size={size.piano} />
            {:else}
              <button class="plain" onclick={() => cycle(current)} title="Ketuk untuk posisi lain">
                <ChordDiagram chord={current.shape} instrument={mode} name={current.name} variant={variants[key(current)] ?? 0} size={size.fret} />
              </button>
            {/if}
          </div>
        </div>
      {/if}
      {#if upcoming.length}
        <div class="next">
          <span class="muted small">Berikutnya</span>
          <div class="row">
            {#each upcoming as it (key(it) + it.name)}
              {#if mode === "piano"}
                <PianoDiagram chord={it.sounding} name={it.name} size={size.nextPiano} />
              {:else}
                <ChordDiagram chord={it.shape} instrument={mode === "all" ? "guitar" : mode} name={it.name} variant={variants[key(it)] ?? 0} size={size.nextFret} />
              {/if}
            {/each}
          </div>
        </div>
      {/if}
    </div>
  {/if}
</aside>

<style>
  .floating {
    position: fixed;
    z-index: 35;
    max-width: calc(100vw - 8px);
    box-shadow: 0 12px 40px rgb(0 0 0 / 0.5);
    background: color-mix(in srgb, var(--bg-2) 94%, transparent);
    backdrop-filter: blur(8px);
  }
  .floating.docked {
    right: 0.8rem;
    bottom: 5.6rem;
  }
  header {
    display: flex;
    align-items: center;
    gap: 0.35rem;
    padding: 0.3rem 0.35rem 0.3rem 0.5rem;
    cursor: grab;
    user-select: none;
    touch-action: none; /* dragging the header must not scroll the page */
    border-bottom: 1px solid var(--line);
  }
  .collapsed header {
    border-bottom: 0;
  }
  header:active {
    cursor: grabbing;
  }
  .grip {
    color: var(--muted);
  }
  .now {
    font-size: 1.15rem;
    color: var(--accent);
  }
  .tabs {
    display: flex;
    gap: 0.2rem;
  }
  .tabs button {
    padding: 0.15em 0.55em;
  }
  .body {
    padding: 0.5rem 0.7rem 0.6rem;
    display: grid;
    gap: 0.4rem;
    justify-items: center;
  }
  .all {
    display: flex;
    align-items: flex-end;
    gap: 0.8rem;
    flex-wrap: wrap;
    justify-content: center;
  }
  .next {
    display: grid;
    gap: 0.15rem;
    justify-items: center;
    border-top: 1px solid var(--line);
    padding-top: 0.35rem;
    width: 100%;
    opacity: 0.75;
  }
  .next .row {
    gap: 0.7rem;
    align-items: flex-end;
  }
  .plain {
    all: unset;
    cursor: pointer;
    display: block;
  }
  p {
    margin: 0.4rem 0;
  }
  @media (max-width: 640px) {
    .floating.docked {
      right: 0.4rem;
      bottom: 9.4rem;
    }
  }
</style>
