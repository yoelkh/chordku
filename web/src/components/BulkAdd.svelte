<script lang="ts">
  // Paste a list of links, get a setlist: a new session ("Sesi 1 24/10/2026") or more songs for
  // an open one. Every link is added in paste order, known songs are reused, and with the
  // switch on each song is separated after its chart is ready. Progress per link comes from the
  // job events, so the dialog can be closed while the queue works.
  import { untrack } from "svelte";
  import { api, onServerEvent, type Job, type Setlist } from "../lib/api";
  import { autoSeparate, setAutoSeparate } from "../lib/autoseparate.svelte";
  import { jobLabel } from "../lib/i18n/id";
  import { nextSessionName, parseLinks } from "../lib/links";
  import { fail, toast } from "../lib/toast.svelte";

  let {
    setlists,
    target = null,
    initialText = "",
    ondone,
    onclose,
  }: {
    setlists: Setlist[];
    /** Add to this setlist; null creates a new session. */
    target?: Setlist | null;
    initialText?: string;
    ondone: (setlistId: number) => void;
    onclose: () => void;
  } = $props();

  let el = $state<HTMLDialogElement>();
  let name = $state(untrack(() => (target ? target.name : nextSessionName(setlists.map((l) => l.name)))));
  let text = $state(untrack(() => initialText));
  const parsed = $derived(parseLinks(text));

  type RowState = "waiting" | "fetching" | "added" | "existing" | "failed";
  interface Row {
    url: string;
    state: RowState;
    title?: string;
    songId?: string;
    error?: string;
    job?: Job;
  }
  let rows = $state<Row[]>([]);
  let running = $state(false);
  let setlistId = $state<number | null>(untrack(() => target?.id ?? null));

  $effect(() => {
    if (el && !el.open) el.showModal();
  });

  // Live job progress for the songs of this batch (download, analysis, separation).
  $effect(() =>
    onServerEvent((e) => {
      if (e.type !== "job") return;
      for (const r of rows) if (r.songId === e.job.song_id) r.job = e.job;
    }),
  );

  const done = $derived(rows.filter((r) => r.state === "added" || r.state === "existing").length);
  const failed = $derived(rows.filter((r) => r.state === "failed").length);

  async function addOne(r: Row) {
    r.state = "fetching";
    r.error = undefined;
    try {
      const res = await api.addUrl(r.url, autoSeparate.on);
      r.songId = res.song.id;
      r.title = res.song.title;
      r.state = res.existing && !res.retried ? "existing" : "added";
      if (res.job) r.job = res.job;
      if (res.warning) warnings.add(res.warning);
    } catch (e) {
      r.state = "failed";
      r.error = e instanceof Error ? e.message : String(e);
    }
  }
  let warnings = new Set<string>();

  /** What the setlist held before this batch: those songs keep their places. */
  let before: string[] = [];
  /** The setlist gets the batch's new songs in paste order, after whatever it already had. */
  async function syncOrder() {
    if (setlistId === null) return;
    const batch = rows.flatMap((r) => (r.songId && !before.includes(r.songId) ? [r.songId] : []));
    await api.orderSetlist(setlistId, [...before, ...batch.filter((id, i) => batch.indexOf(id) === i)]);
  }

  async function start() {
    if (!parsed.urls.length || running) return;
    running = true;
    warnings = new Set();
    try {
      if (setlistId === null) {
        const n = name.trim() || nextSessionName(setlists.map((l) => l.name));
        setlistId = (await api.createSetlist(n)).id;
        ondone(setlistId); // show the new session right away; songs appear as they come in
      }
      before = (await api.setlist(setlistId)).songs.map((s) => s.id);
      rows = parsed.urls.map((url) => ({ url, state: "waiting" as RowState }));
      text = "";
      // Two at a time: metadata lookups are the slow part here; the server limits downloads.
      let next = 0;
      const worker = async () => {
        while (next < rows.length) {
          const r = rows[next++];
          await addOne(r);
          await syncOrder().catch(() => {});
        }
      };
      await Promise.all([worker(), worker()]);
      await syncOrder();
      for (const w of warnings) toast(w, "info", 9000);
      toast(failed ? `${done} lagu masuk, ${failed} gagal (lihat daftar).` : `${done} lagu masuk ke setlist.`, failed ? "error" : "ok");
      ondone(setlistId);
    } catch (e) {
      fail(e);
    } finally {
      running = false;
    }
  }

  async function retry(r: Row) {
    running = true;
    await addOne(r);
    await syncOrder().catch(fail);
    running = false;
    if (setlistId !== null) ondone(setlistId);
  }

  const status = (r: Row) => {
    if (r.state === "waiting") return "menunggu";
    if (r.state === "fetching") return "mengambil info...";
    if (r.state === "failed") return r.error ?? "gagal";
    if (r.job && (r.job.state === "queued" || r.job.state === "running"))
      return jobLabel(r.job) + (r.job.state === "running" && r.job.progress > 0 ? ` ${Math.round(r.job.progress * 100)}%` : "");
    if (r.job?.state === "failed") return `gagal: ${r.job.error ?? ""}`;
    return r.state === "existing" ? "sudah ada di pustaka" : "siap";
  };
</script>

<dialog bind:this={el} class="card" oncancel={(e) => (e.preventDefault(), onclose())}>
  <div class="head row">
    <h3 class="grow">{target ? `Tambah lagu ke ${target.name}` : "Sesi baru"}</h3>
    <button class="ghost icon" onclick={onclose} aria-label="Tutup">✕</button>
  </div>

  {#if !rows.length}
    {#if !target}
      <label class="field">
        <span class="muted small">Nama sesi</span>
        <input type="text" bind:value={name} maxlength="80" />
      </label>
    {/if}
    <label class="field">
      <span class="muted small">Tempel semua tautan (YouTube atau situs lain), satu per baris atau campur dengan teks lain</span>
      <textarea bind:value={text} rows="8" placeholder={"https://youtu.be/...\nhttps://www.youtube.com/watch?v=...\n..."}></textarea>
    </label>
    <div class="row wrap summary">
      <span class="small">
        {#if parsed.urls.length}
          <strong>{parsed.urls.length} tautan</strong> ditemukan{parsed.duplicates ? `, ${parsed.duplicates} duplikat dilewati` : ""}.
        {:else}
          <span class="muted">Belum ada tautan.</span>
        {/if}
      </span>
    </div>
    <label class="row check">
      <input type="checkbox" checked={autoSeparate.on} onchange={(e) => setAutoSeparate(e.currentTarget.checked)} />
      <span>Pisahkan vokal & instrumen otomatis <span class="muted small">(setelah chord jadi; sekitar 12 detik per lagu di GPU)</span></span>
    </label>
    <div class="row end">
      <button class="ghost" onclick={onclose}>Batal</button>
      <button class="primary" onclick={start} disabled={!parsed.urls.length || running}>
        {target ? `Tambahkan ${parsed.urls.length || ""} lagu` : `Buat sesi dan proses ${parsed.urls.length || ""} lagu`}
      </button>
    </div>
  {:else}
    <p class="small">
      {running ? "Memproses" : "Selesai"}: {done}/{rows.length} masuk{failed ? `, ${failed} gagal` : ""}.
      <span class="muted">Unduhan, chord{autoSeparate.on ? ", dan pemisahan" : ""} berjalan di latar; dialog ini boleh ditutup.</span>
    </p>
    <ol class="rows">
      {#each rows as r, i (r.url)}
        <li class:failed={r.state === "failed" || r.job?.state === "failed"} class:ok={(r.state === "added" || r.state === "existing") && (!r.job || r.job.state === "succeeded")}>
          <span class="num mono muted">{i + 1}</span>
          <span class="grow info">
            <span class="ellipsis">{r.title ?? r.url}</span>
            <span class="muted small ellipsis">{status(r)}</span>
          </span>
          {#if r.state === "failed" || r.job?.state === "failed"}<button class="small" onclick={() => retry(r)} disabled={running}>Coba lagi</button>{/if}
        </li>
      {/each}
    </ol>
    <div class="row end">
      <button class="primary" onclick={onclose}>{running ? "Tutup (tetap berjalan)" : "Tutup"}</button>
    </div>
  {/if}
</dialog>

<style>
  dialog {
    border: 1px solid var(--line);
    padding: 1rem 1.1rem;
    width: min(94vw, 40rem);
    color: inherit;
    display: grid;
    gap: 0.75rem;
  }
  dialog:not([open]) {
    display: none;
  }
  dialog::backdrop {
    background: rgb(0 0 0 / 0.55);
  }
  h3 {
    margin: 0;
    font-size: 1.05rem;
  }
  .field {
    display: grid;
    gap: 0.3rem;
  }
  textarea {
    width: 100%;
    resize: vertical;
    background: var(--bg);
    border: 1px solid var(--line);
    border-radius: 8px;
    padding: 0.55em 0.7em;
    font: inherit;
    font-size: 0.9rem;
    color: inherit;
  }
  textarea:focus-visible {
    outline: 2px solid var(--accent);
    outline-offset: 1px;
  }
  .check {
    gap: 0.5rem;
    align-items: flex-start;
  }
  .check input {
    margin-top: 0.25rem;
  }
  .end {
    justify-content: flex-end;
    gap: 0.5rem;
  }
  .rows {
    list-style: none;
    margin: 0;
    padding: 0;
    display: grid;
    gap: 0.25rem;
    max-height: 55vh;
    overflow: auto;
  }
  .rows li {
    display: flex;
    align-items: center;
    gap: 0.6rem;
    padding: 0.4rem 0.5rem;
    border: 1px solid var(--line);
    border-radius: 8px;
  }
  .rows li.ok {
    border-color: color-mix(in srgb, var(--good) 50%, var(--line));
  }
  .rows li.failed {
    border-color: var(--bad);
  }
  .num {
    min-width: 1.4rem;
    text-align: right;
  }
  .info {
    display: grid;
    min-width: 0;
  }
</style>
