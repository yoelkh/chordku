<script lang="ts">
  import { onDestroy, untrack } from "svelte";
  import ChartGrid, { type CellTarget } from "../components/ChartGrid.svelte";
  import ChordDiagram from "../components/ChordDiagram.svelte";
  import ChordEditor from "../components/ChordEditor.svelte";
  import FloatingDiagrams, { type DiagramItem } from "../components/FloatingDiagrams.svelte";
  import Mixer, { type Channel } from "../components/Mixer.svelte";
  import PianoDiagram from "../components/PianoDiagram.svelte";
  import Transport from "../components/Transport.svelte";
  import ViewControls from "../components/ViewControls.svelte";
  import { api, onServerEvent, type Job, type Setlist, type SongDetail } from "../lib/api";
  import { Engine, type TrackSpec } from "../lib/audio/engine";
  import { download, toChordPro, toText } from "../lib/chart/export";
  import { applyEdits, keyAt, segmentAt, type Chart } from "../lib/chart/model";
  import { DEFAULT_PREFS, keySummary, labelAt, normalizePrefs, type ViewPrefs } from "../lib/chart/view";
  import { clock, jobLabel } from "../lib/i18n/id";
  import { classToChord, keyName, noteName, suggestCapo, transposeChord, wrapTranspose, type Key, type Mode } from "../lib/theory";
  import { askConfirm, askText } from "../lib/dialog.svelte";
  import { fail, toast } from "../lib/toast.svelte";

  let { id }: { id: string } = $props();

  let detail = $state<SongDetail | null>(null);
  let missing = $state(false);
  let prefs = $state<ViewPrefs>({ ...DEFAULT_PREFS });
  let jobs = $state<Record<string, Job>>({});

  // ---- data ------------------------------------------------------------------------------
  let first = true;
  async function refresh() {
    try {
      const d = await api.song(id);
      detail = d;
      for (const j of d.jobs) jobs[j.kind] = j;
      if (first) {
        prefs = normalizePrefs(d.prefs);
        first = false;
      }
    } catch (e) {
      if ((e as { status?: number }).status === 404) missing = true;
      else fail(e);
    }
  }
  void refresh();

  const stopEvents = onServerEvent((e) => {
    if (e.type === "job" && e.job.song_id === id) {
      jobs[e.job.kind] = e.job;
      if (e.job.state === "failed") toast(jobLabel(e.job), "error");
    } else if ((e.type === "song" && e.id === id) || e.type === "system") {
      void refresh();
    } else if (e.type === "song_deleted" && e.id === id) {
      location.hash = "#/";
    }
  });

  const chart = $derived<Chart | null>(detail?.analysis ? applyEdits(detail.analysis, detail.edits) : null);
  const activeJob = $derived(Object.values(jobs).find((j) => j.state === "queued" || j.state === "running") ?? null);

  // Persist display preferences (debounced), once loaded.
  let saveTimer = 0;
  $effect(() => {
    const snapshot = JSON.stringify(prefs);
    if (first) return;
    clearTimeout(saveTimer);
    saveTimer = window.setTimeout(() => api.setPrefs(id, JSON.parse(snapshot)).catch(() => {}), 600);
  });

  // ---- audio -----------------------------------------------------------------------------
  const engine = new Engine();
  // Handle for automated checks and for curious users in the console.
  (window as unknown as { chordkuEngine?: Engine }).chordkuEngine = engine;
  let playing = $state(false);
  let positionMs = $state(0);
  let rate = $state(1);
  let loop = $state<{ a: number; b: number } | null>(null);
  let loading = $state<number | null>(null);
  let ready = $state(false);

  type Source = "mix" | "stems" | "light";
  // Four decoded stereo stems of a four minute song are ~370 MB of AudioBuffers, more than a
  // mobile browser tab can be trusted with whatever the phone's RAM: phones default to the
  // two mono stems (~90 MB). The choice stays one tap away.
  const smallDevice =
    /Mobi|Android|iPhone|iPad/i.test(navigator.userAgent) ||
    ((navigator as { deviceMemory?: number }).deviceMemory ?? 8) <= 4 ||
    matchMedia("(max-width: 760px)").matches;
  let source = $state<Source>("mix");
  let sourceChosen = false;
  let channels = $state<Channel[]>([]);

  const stemNames = $derived(new Set(detail?.stems.map((s) => s.name) ?? []));
  const hasMix = $derived(stemNames.has("mix"));
  const hasStems = $derived(["drums", "bass", "other", "vocals"].every((n) => stemNames.has(n)));
  const hasLight = $derived(stemNames.has("vocals_mono") && stemNames.has("backing_mono"));

  function tracksFor(src: Source): TrackSpec[] {
    const names = src === "stems" ? ["vocals", "other", "bass", "drums"] : src === "light" ? ["vocals_mono", "backing_mono"] : ["mix"];
    return names.map((name) => ({ name, url: api.mediaUrl(id, `${name}.opus`) }));
  }

  let loadToken = 0;
  let loadedKey = "";
  async function loadAudio(src: Source) {
    const token = ++loadToken;
    loading = 0;
    try {
      await engine.load(tracksFor(src), (f) => token === loadToken && (loading = f));
      if (token !== loadToken) return;
      channels = engine.trackNames.map((name) => ({ name, gain: 1, muted: false, solo: false }));
      ready = true;
    } catch (e) {
      if (token === loadToken) fail(new Error(`Audio gagal dimuat: ${(e as Error).message}`));
    } finally {
      if (token === loadToken) loading = null;
    }
  }

  // (Re)load when the available files or the chosen source change.
  $effect(() => {
    if (!hasMix) return;
    if (!sourceChosen && hasStems) source = smallDevice && hasLight ? "light" : "stems";
    const want: Source = source === "stems" && !hasStems ? "mix" : source === "light" && !hasLight ? "mix" : source;
    const key = `${want}:${detail?.stems.map((s) => s.name + s.bytes).join(",")}`;
    if (key === loadedKey) return;
    loadedKey = key;
    untrack(() => void loadAudio(want));
  });

  function chooseSource(s: Source) {
    sourceChosen = true;
    source = s;
  }

  $effect(() => {
    const anySolo = channels.some((c) => c.solo);
    for (const c of channels) engine.setGain(c.name, c.muted || (anySolo && !c.solo) ? 0 : c.gain);
  });

  $effect(() => {
    void engine.setTranspose(prefs.pitch_follow ? prefs.transpose : 0);
  });

  let raf = 0;
  const tick = () => {
    positionMs = engine.position() * 1000;
    raf = engine.isPlaying ? requestAnimationFrame(tick) : 0;
  };
  const stopEngine = engine.on(() => {
    playing = engine.isPlaying;
    rate = engine.getRate();
    loop = engine.getLoop();
    positionMs = engine.position() * 1000;
    if (playing && !raf) raf = requestAnimationFrame(tick);
  });

  onDestroy(() => {
    stopEvents();
    stopEngine();
    cancelAnimationFrame(raf);
    clearTimeout(saveTimer);
    void engine.dispose();
  });

  function onkey(e: KeyboardEvent) {
    const t = e.target as HTMLElement;
    if (t.closest("input, select, textarea, [role=dialog]")) return;
    if (e.key === " " && t.tagName !== "BUTTON") {
      e.preventDefault();
      void engine.toggle();
    } else if (e.key === "ArrowLeft") void engine.seek(engine.position() - 5);
    else if (e.key === "ArrowRight") void engine.seek(engine.position() + 5);
    else if (e.key === "[") prefs.transpose = wrapTranspose(prefs.transpose - 1);
    else if (e.key === "]") prefs.transpose = wrapTranspose(prefs.transpose + 1);
  }

  // ---- view helpers ----------------------------------------------------------------------
  const summary = $derived(chart ? keySummary(chart, prefs) : null);

  const capoOptions = $derived.by(() => {
    if (!chart) return [];
    const chords = chart.segments.flatMap((s) => {
      const c = classToChord(s.class);
      return c ? [{ chord: transposeChord(c, prefs.transpose), ms: s.t1_ms - s.t0_ms }] : [];
    });
    const k = chart.key ? { tonic_pc: (chart.key.tonic_pc + prefs.transpose + 12) % 12, mode: chart.key.mode } : null;
    return suggestCapo(chords, k).slice(0, 3);
  });

  // Diagram strip: the chord sounding now, then the next few different ones.
  const upcoming = $derived.by(() => {
    if (!chart) return [];
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
  const nameFor = (index: number) => labelAt(chart!, index, { ...prefs, notation: "names" }).text;

  // Floating cheat-sheet: on by default, remembered per device (a phone and a laptop differ).
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
  const setFloating = (on: boolean) => {
    floating = on;
    localStorage.setItem("chordku.floating.on", on ? "1" : "0");
  };
  const setFloatMode = (m: FloatMode) => {
    floatMode = m;
    localStorage.setItem("chordku.floating.mode", m);
    if (m !== "all") prefs.instrument = m; // keep the toolbar's instrument in step
  };
  const floatItems = $derived.by<DiagramItem[]>(() => {
    if (!chart) return [];
    return upcoming.flatMap((u) => {
      const l = labelAt(chart, u.index, prefs);
      if (!l.shape || !l.sounding) return [];
      const name = prefs.notation === "nashville" ? `${u.text} = ${nameFor(u.index)}` : u.text;
      return [{ name, shape: l.shape, sounding: l.sounding }];
    });
  });

  // ---- edits -----------------------------------------------------------------------------
  let editing = $state(false);
  let follow = $state(true);
  // Drum tab under the chords, remembered per device; only offered once the stem exists.
  let showDrums = $state(stored<string>("chordku.drums.on", "0") === "1");
  const setDrums = (on: boolean) => {
    showDrums = on;
    localStorage.setItem("chordku.drums.on", on ? "1" : "0");
  };
  let target = $state<{ segment: number; range: { t0_ms: number; t1_ms: number }; key: Key | null } | null>(null);

  function openEditor(t: CellTarget) {
    if (!chart) return;
    const seg = chart.segments[t.segment];
    // Tapping the chord itself replaces all of it; tapping a later beat starts a new chord there.
    const t0 = t.isStart ? seg.t0_ms : t.t_ms;
    target = { segment: t.segment, range: { t0_ms: t0, t1_ms: seg.t1_ms }, key: keyAt(chart, t0) };
  }

  async function addEdit(op: Parameters<typeof api.addEdit>[1]["op"], t0: number, t1: number, payload: Record<string, unknown>) {
    try {
      await api.addEdit(id, { op, t0_ms: Math.round(t0), t1_ms: Math.round(t1), payload });
      await refresh();
    } catch (e) {
      fail(e);
    }
  }

  async function saveChord(cls: number) {
    if (!target) return;
    const { range } = target;
    target = null;
    await addEdit("set_chord", range.t0_ms, range.t1_ms, { class: cls });
  }

  async function undoLast() {
    const last = detail?.edits.at(-1);
    if (!last) return;
    try {
      await api.deleteEdit(id, last.id);
      await refresh();
    } catch (e) {
      fail(e);
    }
  }

  async function renameSection(t_ms: number, current: string) {
    const label = await askText("Nama bagian", current, { placeholder: "misalnya Intro, Bait, Reff" });
    if (label) void addEdit("section_rename", t_ms, t_ms, { label });
  }

  function overrideKey(value: string) {
    if (!value) return;
    const [pc, mode] = value.split(":");
    void addEdit("key_override", 0, 0, { tonic_pc: +pc, mode: mode as Mode });
  }

  // ---- actions ---------------------------------------------------------------------------
  let menu = $state(false);
  const run = async (fn: () => Promise<unknown>, done?: string) => {
    menu = false;
    try {
      await fn();
      if (done) toast(done, "ok");
      await refresh();
    } catch (e) {
      fail(e);
    }
  };

  async function editMeta() {
    if (!detail) return;
    const title = await askText("Judul lagu", detail.song.title);
    if (title === null) return;
    const artist = await askText("Artis", detail.song.artist, { ok: "Lanjut", allowEmpty: true });
    if (artist === null) return;
    const tags = await askText("Label (pisahkan dengan koma, boleh kosong)", detail.song.tags.join(", "), { ok: "Simpan", allowEmpty: true, placeholder: "misalnya pujian, lambat" });
    if (tags === null) return;
    await run(() => api.patchSong(id, { title, artist, tags: tags.split(",").map((t) => t.trim()).filter(Boolean) }));
  }

  let setlists = $state<Setlist[]>([]);
  const loadSetlists = () => api.setlists().then((r) => (setlists = r.setlists), fail);
  void loadSetlists();

  async function addToSetlist(value: string) {
    if (!value) return;
    menu = false;
    try {
      let target = Number(value);
      if (value === "new") {
        const name = await askText("Nama setlist baru", "", { placeholder: "misalnya Ibadah Minggu" });
        if (!name) return;
        target = (await api.createSetlist(name)).id;
      }
      setlists = (await api.addToSetlist(target, id)).setlists;
      toast(`Ditambahkan ke setlist ${setlists.find((l) => l.id === target)?.name ?? ""}.`, "ok");
    } catch (e) {
      fail(e);
    }
  }

  async function remove() {
    if (!(await askConfirm("Hapus lagu ini?", { text: "Chord, koreksi, dan stem-nya ikut terhapus.", ok: "Hapus", danger: true }))) return;
    try {
      await api.deleteSong(id);
      location.hash = "#/";
    } catch (e) {
      fail(e);
    }
  }

  const fileBase = $derived(detail ? `${detail.song.artist ? detail.song.artist + " - " : ""}${detail.song.title}` : "chordku");
  function exportAs(kind: "chordpro" | "txt" | "pdf") {
    menu = false;
    if (!chart || !detail) return;
    const meta = { title: detail.song.title, artist: detail.song.artist };
    if (kind === "pdf") {
      const old = document.title;
      document.title = fileBase; // becomes the suggested PDF file name
      print();
      document.title = old;
    } else if (kind === "chordpro") download(`${fileBase}.cho`, toChordPro(chart, prefs, meta));
    else download(`${fileBase}.txt`, toText(chart, prefs, meta));
  }

  const KEY_CHOICES = Array.from({ length: 24 }, (_, i) => ({ pc: i % 12, mode: (i < 12 ? "major" : "minor") as Mode }));
</script>

<svelte:window onkeydown={onkey} />

{#if missing}
  <p class="muted">Lagu tidak ditemukan. <a href="#/">Kembali ke pustaka</a></p>
{:else if !detail}
  <p class="muted">Memuat...</p>
{:else}
  <div class="head row wrap">
    <a class="back no-print" href="#/" aria-label="Kembali ke pustaka">←</a>
    <div class="grow titles">
      <h1 class="ellipsis">{detail.song.title}</h1>
      <div class="muted ellipsis">{detail.song.artist || "Artis tidak diketahui"}</div>
    </div>
    <div class="facts row wrap">
      {#if summary}
        <span class="pill big">Kunci <strong>{summary.shown}</strong>{#if summary.shown !== summary.sounding}<span class="muted">&nbsp;(terdengar {summary.sounding})</span>{/if}</span>
      {/if}
      {#if chart?.bpm}<span class="pill big">{Math.round(chart.bpm)} BPM</span>{/if}
      {#if chart}<span class="pill big">{chart.meter}/4</span>{/if}
      {#if chart && Math.abs(chart.tuning_cents) >= 8}
        <span class="pill big" title="Rekaman ini tidak tepat di A=440 Hz. Analisis sudah menyesuaikannya; setem instrumen Anda sedikit {chart.tuning_cents > 0 ? 'lebih tinggi' : 'lebih rendah'} bila ingin main bersama rekaman.">
          Tuning {chart.tuning_cents > 0 ? "+" : ""}{Math.round(chart.tuning_cents)} sen
        </span>
      {/if}
    </div>
  </div>

  {#if activeJob}
    <div class="card job no-print">
      <div class="row">
        <span class="grow">{jobLabel(activeJob)}{activeJob.kind === "separate" && activeJob.state === "running" ? " (boleh sambil main, ini berjalan di latar)" : ""}</span>
        <button class="ghost small" onclick={() => api.cancelJob(activeJob.id).catch(fail)}>Batalkan</button>
      </div>
      <div class="bar"><i style="width:{Math.round(activeJob.progress * 100)}%"></i></div>
    </div>
  {:else if !chart && jobs.analyze?.state === "failed"}
    <div class="card job failed">
      <strong>Analisis gagal.</strong>
      <span>{jobs.analyze.error}</span>
      <div><button onclick={() => run(() => api.reanalyze(id))}>Coba lagi</button></div>
    </div>
  {/if}

  {#if chart}
    <ViewControls bind:prefs {capoOptions} {floating} {follow} onfloating={setFloating} onfollow={(on) => (follow = on)}>
        <button
          class:on={showDrums && !!detail.drums}
          onclick={() => setDrums(!showDrums)}
          aria-pressed={showDrums && !!detail.drums}
          disabled={!detail.drums}
          title={detail.drums ? "Kick, snare, dan hi-hat per bar, dibaca dari stem drum" : "Pisahkan vokal dan instrumen dulu: tab drum dibaca dari stem drum"}
        >Tab drum</button>
        <button class:on={editing} onclick={() => (editing = !editing)} aria-pressed={editing}>{editing ? "Selesai koreksi" : "Koreksi chord"}</button>
        {#if detail.edits.length}<button class="ghost small" onclick={undoLast} title="Batalkan koreksi terakhir">Urungkan</button>{/if}
        <div class="menu-wrap">
          <button class:on={menu} onclick={() => ((menu = !menu), menu && void loadSetlists())} aria-haspopup="menu" aria-expanded={menu}>Lainnya</button>
          {#if menu}
            <div class="card pop right" role="menu">
              <div class="muted small">Ekspor</div>
              <button class="option" onclick={() => exportAs("pdf")}>PDF (cetak)</button>
              <button class="option" onclick={() => exportAs("chordpro")}>ChordPro (.cho)</button>
              <button class="option" onclick={() => exportAs("txt")}>Teks (.txt)</button>
              <div class="muted small">Ketukan dan kunci</div>
              <label class="option row">Birama
                <select value={chart.meter} onchange={(e) => addEdit("meter_override", 0, 0, { meter: +e.currentTarget.value })}>
                  {#each [2, 3, 4, 5, 6, 7] as m}<option value={m}>{m}/4</option>{/each}
                </select>
              </label>
              <button class="option" onclick={() => addEdit("downbeat_shift", 0, 0, { shift: 1 })}>Geser hitungan "satu" satu ketuk</button>
              <button class="option" onclick={() => addEdit("tempo_scale", 0, 0, { factor: 2 })}>Tempo terbaca setengahnya: gandakan</button>
              <button class="option" onclick={() => addEdit("tempo_scale", 0, 0, { factor: 0.5 })}>Tempo terbaca dobel: bagi dua</button>
              <label class="option row">Kunci
                <select value="" onchange={(e) => overrideKey(e.currentTarget.value)}>
                  <option value="">{chart.key ? keyName(chart.key) : "?"} (ubah)</option>
                  {#each KEY_CHOICES as k}<option value="{k.pc}:{k.mode}">{noteName(k.pc, [1, 3, 5, 8, 10].includes(k.mode === "minor" ? (k.pc + 3) % 12 : k.pc))}{k.mode === "minor" ? "m" : ""}</option>{/each}
                </select>
              </label>
              <div class="muted small">Lagu</div>
              <button class="option" onclick={editMeta}>Ubah judul, artis, label</button>
              <label class="option row">Setlist
                <select value="" onchange={(e) => addToSetlist(e.currentTarget.value)}>
                  <option value="">Tambahkan ke...</option>
                  {#each setlists as l (l.id)}<option value={l.id}>{l.name}</option>{/each}
                  <option value="new">Setlist baru...</option>
                </select>
              </label>
              {#if hasStems}
                <button class="option" onclick={() => run(() => api.deleteStems(id), "Stem dihapus.")}>Hapus stem (hemat ruang)</button>
              {/if}
              {#if detail.edits.length}
                <button class="option" onclick={async () => (await askConfirm("Buang semua koreksi pada lagu ini?", { ok: "Buang", danger: true })) && run(() => api.clearEdits(id))}>Buang semua koreksi ({detail.edits.length})</button>
              {/if}
              <button class="option" onclick={() => run(() => api.reanalyze(id), "Analisis ulang dimulai.")}>Analisis ulang</button>
              <button class="option danger" onclick={remove}>Hapus lagu</button>
            </div>
          {/if}
        </div>
    </ViewControls>

    {#if showDrums && detail.drums}
      <p class="muted small legend no-print">Tab drum per bar, 16 langkah: <span class="sw hat"></span> hi-hat (redup = diperkirakan dari pola bar) · <span class="sw snare"></span> snare · <span class="sw kick"></span> kick. Perkiraan dari stem drum; tom dan simbal bisa terbaca sebagai kick atau hi-hat.</p>
    {/if}
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

    <div class="audio card no-print">
      <div class="row wrap">
        <strong class="small">Audio</strong>
        <button class="small" class:on={source === "mix"} onclick={() => chooseSource("mix")} disabled={!hasMix}>Lagu asli</button>
        {#if hasStems}
          <button class="small" class:on={source === "stems"} onclick={() => chooseSource("stems")}>4 stem</button>
          <button class="small" class:on={source === "light"} onclick={() => chooseSource("light")} title="Vokal + iringan mono: hemat memori untuk HP">2 stem (ringan)</button>
        {:else if !jobs.separate || !["queued", "running"].includes(jobs.separate.state)}
          <button class="small" onclick={() => run(() => api.separate(id), "Pemisahan stem dimulai. Butuh beberapa menit.")} disabled={!hasMix}>
            Pisahkan vokal dan instrumen
          </button>
          <span class="muted small">untuk membisukan vokal atau berlatih dengan satu instrumen</span>
        {/if}
        {#if !hasMix && !activeJob}<span class="muted small">Audio pemutar belum tersedia.</span>{/if}
      </div>
      {#if channels.length > 1}
        <Mixer {channels} onchange={(c) => (channels = c)} />
      {/if}
    </div>

    <div class="print-head">
      <h1>{detail.song.title}</h1>
      <div>{detail.song.artist}</div>
      <div>
        {#if summary}Kunci {summary.shown}{prefs.capo > 0 ? ` (capo ${prefs.capo}, terdengar ${summary.sounding})` : ""}{/if}
        {#if chart.bpm} · {Math.round(chart.bpm)} BPM{/if} · {chart.meter}/4 · {clock(chart.duration_ms)}
      </div>
    </div>

    <ChartGrid
      {chart}
      {prefs}
      {positionMs}
      {editing}
      {follow}
      {loop}
      drums={showDrums ? detail.drums : null}
      onseek={(ms) => engine.seek(ms / 1000)}
      onedit={openEditor}
      onloop={(r) => engine.setLoop(r)}
      onrename={renameSection}
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
      ontoggle={() => engine.toggle()}
      onseek={(ms) => engine.seek(ms / 1000)}
      onrate={(r) => engine.setRate(r)}
      onloop={(l) => engine.setLoop(l)}
    />

    {#if target}
      <ChordEditor
        segment={chart.segments[target.segment]}
        range={target.range}
        keyNow={target.key}
        {prefs}
        onsave={saveChord}
        onclose={() => (target = null)}
        onaudition={(pcs) => engine.audition(pcs)}
      />
    {/if}
  {/if}
{/if}

<style>
  .head {
    gap: 0.8rem;
    margin-bottom: 0.9rem;
  }
  .back {
    font-size: 1.4rem;
    color: var(--muted);
    padding: 0 0.3rem;
  }
  .titles h1 {
    font-size: clamp(1.15rem, 3vw, 1.6rem);
  }
  .facts {
    gap: 0.35rem;
  }
  .pill.big {
    font-size: 0.9rem;
    padding: 0.25em 0.75em;
    color: var(--text);
  }
  .job {
    padding: 0.7rem 0.9rem;
    display: grid;
    gap: 0.5rem;
    margin-bottom: 0.8rem;
  }
  .job.failed {
    border-color: var(--bad);
  }
  .menu-wrap {
    position: relative;
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
  .pop.right {
    left: auto;
    right: 0;
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
  .legend {
    margin: 0.5rem 0 0;
  }
  .sw {
    display: inline-block;
    width: 0.8em;
    height: 0.55em;
    border-radius: 3px;
    vertical-align: middle;
  }
  .sw.hat {
    background: var(--muted);
  }
  .sw.snare {
    background: var(--good);
  }
  .sw.kick {
    background: var(--accent);
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
  .audio {
    padding: 0.6rem 0.8rem;
    display: grid;
    gap: 0.6rem;
    margin-bottom: 1rem;
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
