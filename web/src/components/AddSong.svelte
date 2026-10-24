<script lang="ts">
  import { api } from "../lib/api";
  import { autoSeparate, setAutoSeparate } from "../lib/autoseparate.svelte";
  import { parseLinks } from "../lib/links";
  import { fail, toast } from "../lib/toast.svelte";
  import Recorder from "./Recorder.svelte";

  let {
    onadded,
    onbulk,
    hint = "",
  }: { onadded: (songId: string, existing: boolean) => void; onbulk?: (text: string) => void; hint?: string } = $props();

  // Several links pasted into the single-link box: hand them to the bulk dialog instead.
  function onpaste(e: ClipboardEvent) {
    const text = e.clipboardData?.getData("text") ?? "";
    if (onbulk && parseLinks(text).urls.length > 1) {
      e.preventDefault();
      onbulk(text);
    }
  }
  const warn = (w?: string | null) => w && toast(w, "info", 9000);

  let mode = $state<"url" | "file" | "mic">("url");
  let url = $state("");
  let busy = $state(false);
  let uploadProgress = $state<number | null>(null);
  let dragging = $state(false);
  let fileInput = $state<HTMLInputElement>();

  async function submitUrl(e: Event) {
    e.preventDefault();
    const value = url.trim();
    if (!value || busy) return;
    busy = true;
    try {
      const many = parseLinks(value);
      if (onbulk && many.urls.length > 1) {
        onbulk(value);
        url = "";
        return;
      }
      const res = await api.addUrl(value, autoSeparate.on);
      warn(res.warning);
      if (res.existing) toast("Lagu ini sudah ada di pustaka.", "info");
      url = "";
      onadded(res.song.id, res.existing);
    } catch (err) {
      fail(err);
    } finally {
      busy = false;
    }
  }

  async function uploadFiles(files: FileList | File[] | null) {
    if (!files || busy) return;
    busy = true;
    try {
      for (const file of Array.from(files)) {
        uploadProgress = 0;
        const res = await api.upload(file, { kind: "file", separate: autoSeparate.on }, (f) => (uploadProgress = f));
        warn(res.warning);
        if (res.existing) toast(`"${file.name}" sudah ada di pustaka.`, "info");
        onadded(res.song.id, res.existing);
      }
    } catch (err) {
      fail(err);
    } finally {
      busy = false;
      uploadProgress = null;
      if (fileInput) fileInput.value = "";
    }
  }

  async function uploadRecording(blob: Blob, title: string) {
    busy = true;
    try {
      uploadProgress = 0;
      const ext = blob.type.includes("mp4") ? "m4a" : blob.type.includes("ogg") ? "ogg" : "webm";
      const res = await api.upload(blob, { kind: "mic", title, filename: `rekaman.${ext}`, separate: autoSeparate.on }, (f) => (uploadProgress = f));
      warn(res.warning);
      onadded(res.song.id, res.existing);
    } catch (err) {
      fail(err);
    } finally {
      busy = false;
      uploadProgress = null;
    }
  }

  function onDrop(e: DragEvent) {
    e.preventDefault();
    dragging = false;
    void uploadFiles(e.dataTransfer?.files ?? null);
  }
</script>

<section class="card add">
  {#if hint}<p class="hint setlist">{hint}</p>{/if}
  <div class="tabs" role="tablist">
    <button role="tab" aria-selected={mode === "url"} class:on={mode === "url"} onclick={() => (mode = "url")}>Tautan</button>
    <button role="tab" aria-selected={mode === "file"} class:on={mode === "file"} onclick={() => (mode = "file")}>File</button>
    <button role="tab" aria-selected={mode === "mic"} class:on={mode === "mic"} onclick={() => (mode = "mic")}>Rekam</button>
  </div>

  {#if mode === "url"}
    <form class="row" onsubmit={submitUrl}>
      <input class="grow" type="text" inputmode="url" placeholder="Tempel tautan YouTube, SoundCloud, TikTok, dan lainnya (bisa banyak sekaligus)" bind:value={url} {onpaste} disabled={busy} required />
      <button class="primary" disabled={busy || !url.trim()}>{busy ? "Mengambil info..." : "Cari chord"}</button>
    </form>
    <p class="muted small hint">Audio diambil dengan yt-dlp untuk dipelajari sendiri. Playlist dan siaran langsung tidak didukung. Tempel banyak tautan sekaligus untuk membuat sesi.</p>
  {:else if mode === "file"}
    <div
      class="drop"
      class:dragging
      role="button"
      tabindex="0"
      ondragover={(e) => (e.preventDefault(), (dragging = true))}
      ondragleave={() => (dragging = false)}
      ondrop={onDrop}
      onclick={() => fileInput?.click()}
      onkeydown={(e) => (e.key === "Enter" || e.key === " ") && fileInput?.click()}
    >
      <strong>Pilih file audio atau video</strong>
      <span class="muted small">atau seret ke sini. MP3, WAV, FLAC, M4A, OGG, MP4, dan lainnya.</span>
      <input bind:this={fileInput} type="file" accept="audio/*,video/*,.flac,.m4a,.opus,.mkv" multiple hidden onchange={(e) => uploadFiles(e.currentTarget.files)} />
    </div>
  {:else}
    <Recorder onrecorded={uploadRecording} disabled={busy} />
  {/if}

  <label class="row small auto">
    <input type="checkbox" checked={autoSeparate.on} onchange={(e) => setAutoSeparate(e.currentTarget.checked)} />
    Pisahkan vokal & instrumen otomatis
  </label>

  {#if uploadProgress !== null}
    <div class="bar" aria-label="Kemajuan unggah"><i style="width:{Math.round(uploadProgress * 100)}%"></i></div>
  {/if}
</section>

<style>
  .add {
    padding: 0.9rem;
    display: grid;
    gap: 0.7rem;
  }
  .tabs {
    display: flex;
    gap: 0.35rem;
  }
  .tabs button {
    padding: 0.3em 0.9em;
  }
  .auto {
    gap: 0.4rem;
    color: var(--muted);
  }
  .hint.setlist {
    color: var(--accent);
    font-weight: 600;
  }
  .hint {
    margin: 0;
  }
  .drop {
    display: grid;
    gap: 0.2rem;
    place-items: center;
    text-align: center;
    padding: 1.4rem 1rem;
    border: 2px dashed var(--line);
    border-radius: var(--radius);
    cursor: pointer;
  }
  .drop.dragging,
  .drop:hover {
    border-color: var(--accent);
  }
  @media (max-width: 560px) {
    form.row {
      flex-direction: column;
      align-items: stretch;
    }
  }
</style>
