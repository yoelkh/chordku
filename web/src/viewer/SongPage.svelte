<script lang="ts">
  // One song in the standalone viewer: the same chart, toolbar, diagrams and transport as
  // the app, driven by the embedded YouTube video (when the song came from YouTube) or by a
  // silent clock. Readers' transpose / capo choices are kept in their own browser.
  import { onDestroy, untrack } from "svelte";
  import ChartGrid from "../components/ChartGrid.svelte";
  import ChordDiagram from "../components/ChordDiagram.svelte";
  import FloatingDiagrams, { type DiagramItem } from "../components/FloatingDiagrams.svelte";
  import PianoDiagram from "../components/PianoDiagram.svelte";
  import Transport from "../components/Transport.svelte";
  import ViewControls from "../components/ViewControls.svelte";
  import { download, toChordPro, toText } from "../lib/chart/export";
  import { applyEdits, segmentAt, type Chart } from "../lib/chart/model";
  import { DEFAULT_PREFS, keySummary, labelAt, normalizePrefs, type ViewPrefs } from "../lib/chart/view";
  import { clock } from "../lib/i18n/id";
  import { classToChord, suggestCapo, transposeChord, wrapTranspose } from "../lib/theory";
  import { Engine } from "../lib/audio/engine";
  import { EngineClock, VirtualClock, YouTubeClock, type Clock } from "./clock";
  import { youtubeId, type ViewerSong } from "./data";

  let { song, setlist, index, total, onnav }: { song: ViewerSong; setlist: { id: number; name: string }; index: number; total: number; onnav: (i: number) => void } = $props();

  // The page is keyed by song index (see Viewer.svelte), so the props never change underneath it.
  const s = untrack(() => song);
  const list = untrack(() => setlist);
  const chart: Chart = applyEdits(s.analysis, s.edits);

  // ---- per-reader prefs -------------------------------------------------------------------
  const prefsKey = `chordku.viewer.${list.id}.${s.id}`;
  const readStored = (): ViewPrefs | null => {
    try {
      const raw = localStorage.getItem(prefsKey);
      return raw ? normalizePrefs(JSON.parse(raw)) : null;
    } catch {
      return null;
    }
  };
  // With audio embedded the player follows transpose like the app does; without audio the flag is moot.
  let prefs = $state<ViewPrefs>(readStored() ?? { ...normalizePrefs(s.prefs), pitch_follow: !!s.audio?.mix });
  $effect(() => {
    const json = JSON.stringify(prefs);
    try {
      localStorage.setItem(prefsKey, json);
    } catch {
      /* private mode: prefs live for the session only */
    }
  });

  // ---- time source ------------------------------------------------------------------------
  const audioUrl = s.audio?.mix ?? null;
  // With audio embedded the page plays like the app; the video is only a fallback for pages
  // exported without it.
  const videoId = audioUrl ? null : youtubeId(s.src_url);
  let playing = $state(false);
  let positionMs = $state(0);
  let rate = $state(1);
  let loop = $state<{ a: number; b: number } | null>(null);
  let ready = $state(!videoId && !audioUrl);
  let loading = $state<number | null>(null);
  let mode = $state<"audio" | "youtube" | "virtual">(audioUrl ? "audio" : videoId ? "youtube" : "virtual");
  let engine: Engine | null = null;
  let note = $state("");
  let videoEl = $state<HTMLElement>();
  let showVideo = $state(true);
  let source: Clock | null = null;
  let stopSource: (() => void) | null = null;
  let raf = 0;

  const tick = () => {
    if (!source) return;
    source.tick();
    positionMs = source.position() * 1000;
    raf = source.isPlaying ? requestAnimationFrame(tick) : 0;
  };
  function attach(c: Clock) {
    stopSource?.();
    source?.dispose();
    source = c;
    stopSource = c.on(() => {
      playing = c.isPlaying;
      rate = c.getRate();
      loop = c.getLoop();
      positionMs = c.position() * 1000;
      if (c.kind === "youtube") ready = (c as YouTubeClock).ready;
      if (playing && !raf) raf = requestAnimationFrame(tick);
    });
  }
  function useVirtual(why?: string) {
    mode = "virtual";
    ready = true;
    if (why) note = `${why}. Chart berjalan dengan penghitung tempo (tanpa suara).`;
    attach(new VirtualClock(chart.duration_ms / 1000));
  }
  async function useAudio(url: string) {
    engine = new Engine();
    // Handle for automated checks and for curious users in the console.
    (window as unknown as { chordkuEngine?: Engine }).chordkuEngine = engine;
    attach(new EngineClock(engine));
    loading = 0;
    try {
      await engine.load([{ name: "mix", url }], (f) => (loading = f));
      ready = true;
      void engine.setTranspose(untrack(shift)); // the engine did not exist when the effect first ran
    } catch (e) {
      useVirtual(`Audio gagal dimuat (${(e as Error).message})`);
    } finally {
      loading = null;
    }
  }
  // Read the prefs before the engine check: with `engine?.` short-circuiting first, nothing
  // would be tracked and the effect would never run again.
  const shift = () => (prefs.pitch_follow ? prefs.transpose : 0);
  $effect(() => {
    const semitones = shift();
    void engine?.setTranspose(semitones);
  });
  $effect(() => {
    if (audioUrl && !source) {
      untrack(() => void useAudio(audioUrl));
    } else if (videoId && videoEl && !source) {
      const el = videoEl;
      untrack(() => attach(new YouTubeClock(el, videoId, (why) => useVirtual(why))));
    } else if (!videoId && !source) {
      untrack(() => useVirtual());
    }
  });
  onDestroy(() => {
    stopSource?.();
    source?.dispose();
    cancelAnimationFrame(raf);
    wakeLock?.release().catch(() => {});
  });

  // Keep the screen on while a song is open: nobody wants to touch the phone mid-song.
  let wakeLock: { release(): Promise<void> } | null = null;
  const requestWake = async () => {
    try {
      wakeLock = (await (navigator as { wakeLock?: { request(t: "screen"): Promise<{ release(): Promise<void> }> } }).wakeLock?.request("screen")) ?? null;
    } catch {
      wakeLock = null;
    }
  };
  $effect(() => {
    void requestWake();
    const onVis = () => document.visibilityState === "visible" && void requestWake();
    document.addEventListener("visibilitychange", onVis);
    return () => document.removeEventListener("visibilitychange", onVis);
  });

  function onkey(e: KeyboardEvent) {
    const t = e.target as HTMLElement;
    if (t.closest("input, select, textarea, [role=dialog], iframe")) return;
    if (e.key === " ") {
      e.preventDefault();
      source?.toggle();
    } else if (e.key === "ArrowLeft") source?.seek((source.position() ?? 0) - 5);
    else if (e.key === "ArrowRight") source?.seek((source.position() ?? 0) + 5);
    else if (e.key === "[") prefs.transpose = wrapTranspose(prefs.transpose - 1);
    else if (e.key === "]") prefs.transpose = wrapTranspose(prefs.transpose + 1);
  }

  // ---- view helpers (same as the app's song page) -----------------------------------------
  const summary = $derived(keySummary(chart, prefs));
  const capoOptions = $derived.by(() => {
    const chords = chart.segments.flatMap((s) => {
      const c = classToChord(s.class);
      return c ? [{ chord: transposeChord(c, prefs.transpose), ms: s.t1_ms - s.t0_ms }] : [];
    });
    const k = chart.key ? { tonic_pc: (chart.key.tonic_pc + prefs.transpose + 12) % 12, mode: chart.key.mode } : null;
    return suggestCapo(chords, k).slice(0, 3);
  });
  const upcoming = $derived.by(() => {
    const start = Math.max(0, segmentAt(chart.segments, positionMs));
    const seen = new Set<string>();
    const out: { text: string; index: number }[] = [];
    for (let i = start; i < chart.segments.length && out.length < 4; i++) {
      const l = labelAt(chart, i, prefs);
      if (!l.shape || seen.has(l.text)) continue;
      seen.add(l.text);
      out.push({ text: l.text, index: i });
    }
    return out;
  });
  const nameFor = (i: number) => labelAt(chart, i, { ...prefs, notation: "names" }).text;

  type FloatMode = "guitar" | "ukulele" | "piano" | "all";
  const stored = <T extends string>(key: string, fallback: T): T => {
    try {
      return (localStorage.getItem(key) as T | null) ?? fallback;
    } catch {
      return fallback;
    }
  };
  let floating = $state(stored("chordku.floating.on", "1") === "1");
  let floatMode = $state<FloatMode | "">(stored<FloatMode | "">("chordku.floating.mode", ""));
  let follow = $state(stored("chordku.viewer.follow", "1") === "1");
  const setFloating = (on: boolean) => {
    floating = on;
    try {
      localStorage.setItem("chordku.floating.on", on ? "1" : "0");
    } catch {}
  };
  const setFloatMode = (m: FloatMode) => {
    floatMode = m;
    try {
      localStorage.setItem("chordku.floating.mode", m);
    } catch {}
    if (m !== "all") prefs.instrument = m;
  };
  const setFollow = (on: boolean) => {
    follow = on;
    try {
      localStorage.setItem("chordku.viewer.follow", on ? "1" : "0");
    } catch {}
  };
  const floatItems = $derived.by<DiagramItem[]>(() =>
    upcoming.flatMap((u) => {
      const l = labelAt(chart, u.index, prefs);
      if (!l.shape || !l.sounding) return [];
      const name = prefs.notation === "nashville" ? `${u.text} = ${nameFor(u.index)}` : u.text;
      return [{ name, shape: l.shape, sounding: l.sounding }];
    }),
  );

  // ---- export -----------------------------------------------------------------------------
  let menu = $state(false);
  const meta = $derived({ title: song.title, artist: song.artist });
  const fileBase = $derived(song.title.replace(/[\\/:*?"<>|]+/g, " ").trim() || "chord");
  function exportAs(kind: "chordpro" | "txt" | "pdf") {
    menu = false;
    if (kind === "pdf") {
      const old = document.title;
      document.title = fileBase;
      window.print();
      document.title = old;
    } else if (kind === "chordpro") download(`${fileBase}.cho`, toChordPro(chart, prefs, meta));
    else download(`${fileBase}.txt`, toText(chart, prefs, meta));
  }
</script>

<svelte:window onkeydown={onkey} />

<div class="head row wrap">
  <div class="titles grow">
    <h1>{song.title}</h1>
    <div class="muted">{song.artist || "Artis tidak diketahui"}</div>
  </div>
  <div class="facts row wrap">
    {#if summary}<span class="pill big">Kunci <strong>{summary.shown}</strong>{prefs.capo > 0 ? ` · terdengar ${summary.sounding}` : ""}</span>{/if}
    {#if chart.bpm}<span class="pill big">{Math.round(chart.bpm * rate)} BPM</span>{/if}
    <span class="pill big">{chart.meter}/4</span>
  </div>
</div>

<ViewControls bind:prefs {capoOptions} {floating} {follow} showPitchFollow={mode === "audio"} onfloating={setFloating} onfollow={setFollow}>
  {#if videoId && mode === "youtube"}
    <button class:on={showVideo} onclick={() => (showVideo = !showVideo)} aria-pressed={showVideo}>Video</button>
  {/if}
  <div class="menu-wrap">
    <button class:on={menu} onclick={() => (menu = !menu)} aria-haspopup="menu" aria-expanded={menu}>Ekspor</button>
    {#if menu}
      <div class="card pop right" role="menu">
        <button class="option" onclick={() => exportAs("pdf")}>PDF (cetak)</button>
        <button class="option" onclick={() => exportAs("chordpro")}>ChordPro (.cho)</button>
        <button class="option" onclick={() => exportAs("txt")}>Teks (.txt)</button>
      </div>
    {/if}
  </div>
</ViewControls>

{#if videoId && mode === "youtube"}
  <div class="video card no-print" class:collapsed={!showVideo}>
    <div class="frame"><div bind:this={videoEl}></div></div>
    {#if !ready}<p class="muted small">Memuat video YouTube...</p>{/if}
  </div>
{/if}
{#if note}<p class="muted small note no-print">{note}</p>{/if}

{#if floating}
  <FloatingDiagrams items={floatItems} mode={floatMode === "all" ? "all" : prefs.instrument} onmode={setFloatMode} onclose={() => setFloating(false)} />
{/if}

<div class="strip no-print" class:hidden={floating}>
  {#each upcoming as u, i (u.index)}
    {@const l = labelAt(chart, u.index, prefs)}
    <div class="diagram" class:current={i === 0}>
      {#if prefs.instrument === "piano"}
        {#if l.sounding}<PianoDiagram chord={l.sounding} name={u.text} size={i === 0 ? 170 : 130} />{/if}
      {:else if l.shape}
        <ChordDiagram chord={l.shape} instrument={prefs.instrument} name={prefs.notation === "nashville" ? `${u.text} = ${nameFor(u.index)}` : u.text} size={i === 0 ? 104 : 80} />
      {/if}
    </div>
  {/each}
</div>

<div class="print-head">
  <h1>{song.title}</h1>
  <div>{song.artist}</div>
  <div>
    {#if summary}Kunci {summary.shown}{prefs.capo > 0 ? ` (capo ${prefs.capo}, terdengar ${summary.sounding})` : ""}{/if}
    {#if chart.bpm} · {Math.round(chart.bpm)} BPM{/if} · {chart.meter}/4 · {clock(chart.duration_ms)}
  </div>
</div>

<ChartGrid
  {chart}
  {prefs}
  {positionMs}
  editing={false}
  {follow}
  {loop}
  onseek={(ms) => source?.seek(ms / 1000)}
  onedit={() => {}}
  onloop={(r) => source?.setLoop(r)}
  onrename={() => {}}
/>

<Transport
  {playing}
  {ready}
  {loading}
  {positionMs}
  durationMs={chart.duration_ms}
  {rate}
  {loop}
  sections={chart.sections}
  ontoggle={() => source?.toggle()}
  onseek={(ms) => source?.seek(ms / 1000)}
  onrate={(r) => source?.setRate(r)}
  onloop={(l) => source?.setLoop(l)}
/>

<div class="nav row no-print">
  <button onclick={() => onnav(index - 1)} disabled={index === 0}>‹ Sebelumnya</button>
  <a class="grow center muted small" href="#/">Daftar lagu</a>
  <button onclick={() => onnav(index + 1)} disabled={index === total - 1}>Berikutnya ›</button>
</div>

<style>
  .head {
    gap: 0.8rem;
    margin-bottom: 0.9rem;
  }
  .titles h1 {
    font-size: clamp(1.15rem, 3vw, 1.6rem);
    margin: 0;
  }
  .facts {
    gap: 0.35rem;
  }
  .pill.big {
    font-size: 0.9rem;
    padding: 0.25em 0.75em;
    color: var(--text);
  }
  .menu-wrap {
    position: relative;
  }
  .pop {
    position: absolute;
    top: calc(100% + 0.4rem);
    right: 0;
    z-index: 40;
    width: max-content;
    min-width: 12rem;
    padding: 0.5rem;
    display: grid;
    gap: 0.25rem;
    box-shadow: 0 14px 40px rgb(0 0 0 / 0.5);
  }
  .option {
    text-align: left;
    background: transparent;
    border-color: transparent;
  }
  .option:hover {
    background: var(--bg-3);
  }
  .video {
    margin: 0.8rem 0;
    padding: 0.5rem;
  }
  .frame {
    aspect-ratio: 16 / 9;
    max-height: 40vh;
    margin: 0 auto;
  }
  .frame :global(iframe) {
    width: 100%;
    height: 100%;
    border: 0;
    border-radius: 8px;
  }
  .video.collapsed {
    height: 0;
    padding: 0;
    margin: 0;
    overflow: hidden;
    border: 0;
  }
  .note {
    margin: 0.4rem 0;
  }
  .strip {
    display: flex;
    align-items: flex-end;
    gap: 1rem;
    min-height: 9.2rem;
    padding: 0.8rem 0.2rem 0.4rem;
    overflow-x: auto;
  }
  .strip.hidden {
    display: none;
  }
  .diagram {
    opacity: 0.55;
  }
  .diagram.current {
    opacity: 1;
  }
  .nav {
    margin: 1.2rem 0 0;
    gap: 0.6rem;
  }
  .center {
    text-align: center;
  }
  .print-head {
    display: none;
  }
  @media print {
    .print-head {
      display: block;
      margin-bottom: 6mm;
    }
    .print-head h1 {
      font-size: 18pt;
    }
    .head {
      display: none;
    }
  }
</style>
