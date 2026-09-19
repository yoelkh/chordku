<script lang="ts">
  import AccelCard from "../components/AccelCard.svelte";
  import LanCard from "../components/LanCard.svelte";
  import { api, onServerEvent, type SystemInfo } from "../lib/api";
  import { bytes } from "../lib/i18n/id";
  import { fail, toast } from "../lib/toast.svelte";

  let info = $state<SystemInfo | null>(null);
  let progress = $state<Record<string, number>>({});
  let updating = $state(false);

  // From a phone on the LAN the server refuses administrative endpoints (403) by design.
  let laptopOnly = $state(false);

  async function refresh() {
    try {
      info = await api.system();
    } catch (e) {
      if ((e as { status?: number }).status === 403) laptopOnly = true;
      else fail(e);
    }
  }
  void refresh();

  $effect(() =>
    onServerEvent((e) => {
      if (e.type === "model") {
        if (e.done) {
          delete progress[e.id];
          if (e.error) toast(`Unduhan model gagal: ${e.error}`, "error");
          else toast("Model terpasang.", "ok");
          void refresh();
        } else {
          progress[e.id] = e.progress;
        }
      } else if (e.type === "system") {
        void refresh();
      }
    }),
  );

  async function downloadModel(id: string) {
    try {
      progress[id] = 0;
      await api.downloadModel(id);
    } catch (e) {
      delete progress[id];
      fail(e);
    }
  }

  async function updateYtdlp() {
    updating = true;
    try {
      const res = await api.updateYtdlp();
      toast(`yt-dlp ${res.version} terpasang.`, "ok");
      await refresh();
    } catch (e) {
      fail(e);
    } finally {
      updating = false;
    }
  }
</script>

<h1>Pengaturan</h1>

{#if laptopOnly}
  <p class="muted">Model, yt-dlp, dan akses perangkat hanya bisa diatur dari laptop tempat ChordKu berjalan.</p>
{:else if !info}
  <p class="muted">Memuat...</p>
{:else}
  <section class="card">
    <h2>Model</h2>
    <p class="muted small">Semua analisis berjalan di laptop ini. Tidak ada audio yang dikirim ke internet.</p>
    {#each info.models as m (m.id)}
      <div class="item">
        <div class="grow">
          <div class="row wrap">
            <strong>{m.title}</strong>
            <span class="pill" class:ok={m.installed} class:need={!m.installed && m.required}>{m.installed ? "Terpasang" : m.required ? "Wajib, belum ada" : "Opsional"}</span>
          </div>
          <div class="muted small">Lisensi: {m.license}. Sumber: <a href={m.source} target="_blank" rel="noreferrer">{m.source.replace("https://", "")}</a></div>
          {#if !m.installed && !m.downloadable}
            <div class="small warn">Model ini dibuat lokal satu kali: jalankan <code>tools\setup.ps1</code> di folder proyek.</div>
          {/if}
          {#if progress[m.id] !== undefined}
            <div class="bar"><i style="width:{Math.round(progress[m.id] * 100)}%"></i></div>
          {/if}
        </div>
        {#if !m.installed && m.downloadable}
          <button class="primary" onclick={() => downloadModel(m.id)} disabled={progress[m.id] !== undefined}>
            {progress[m.id] !== undefined ? `${Math.round(progress[m.id] * 100)}%` : `Unduh (${bytes(m.download_bytes)})`}
          </button>
        {/if}
      </div>
    {/each}
  </section>

  <section class="card">
    <h2>Pengambil audio (yt-dlp)</h2>
    <div class="item">
      <div class="grow">
        {#if info.ytdlp.installed}
          <div>Versi <span class="mono">{info.ytdlp.version ?? "?"}</span></div>
        {:else}
          <div>Belum terpasang. Akan dipasang otomatis saat tautan pertama ditambahkan.</div>
        {/if}
        <div class="muted small">
          Runtime JavaScript untuk YouTube:
          {#if info.ytdlp.js_runtime}<span class="mono">{info.ytdlp.js_runtime}</span>{:else}<span class="warn">tidak ditemukan. Pasang Node.js 22+ atau Deno agar YouTube tetap berfungsi.</span>{/if}
        </div>
        <div class="muted small">Jika tautan YouTube tiba-tiba gagal, perbarui yt-dlp. YouTube sering berubah.</div>
      </div>
      <button onclick={updateYtdlp} disabled={updating}>{updating ? "Memperbarui..." : info.ytdlp.installed ? "Perbarui" : "Pasang"}</button>
    </div>
  </section>

  <AccelCard {info} onchanged={refresh} />

  <LanCard />

  <section class="card">
    <h2>Sistem</h2>
    <dl>
      <dt>Versi</dt><dd class="mono">ChordKu {info.version}</dd>
      <dt>Folder data</dt><dd class="mono wrapany">{info.data_dir}</dd>
      <dt>Ukuran pustaka</dt><dd>{bytes(info.media_bytes)}</dd>
      <dt>RAM bebas</dt><dd>{info.free_memory ? bytes(info.free_memory) : "?"} <span class="muted small">(pisah stem butuh sekitar 2,5 GB)</span></dd>
      <dt>Batas durasi</dt><dd>{Math.round(info.max_duration_secs / 60)} menit</dd>
      <dt>Mesin inferensi</dt><dd class="mono small wrapany">{info.onnxruntime}</dd>
    </dl>
  </section>

  <section class="card">
    <h2>Pintasan</h2>
    <dl>
      <dt><kbd>Spasi</kbd></dt><dd>Putar atau jeda</dd>
      <dt><kbd>←</kbd> <kbd>→</kbd></dt><dd>Mundur atau maju 5 detik</dd>
      <dt><kbd>[</kbd> <kbd>]</kbd></dt><dd>Transpose turun atau naik</dd>
    </dl>
  </section>
{/if}

<style>
  h1 {
    margin-bottom: 1rem;
  }
  section {
    padding: 1rem;
    margin-bottom: 0.9rem;
    display: grid;
    gap: 0.6rem;
  }
  h2 {
    font-size: 1.05rem;
  }
  section p {
    margin: 0;
  }
  .item {
    display: flex;
    gap: 0.9rem;
    align-items: center;
    padding-top: 0.6rem;
    border-top: 1px solid var(--line);
  }
  .item .grow {
    display: grid;
    gap: 0.25rem;
  }
  .pill.ok {
    color: var(--good);
    border-color: var(--good);
  }
  .pill.need,
  .warn {
    color: var(--bad);
    border-color: var(--bad);
  }
  dl {
    display: grid;
    grid-template-columns: max-content 1fr;
    gap: 0.35rem 1rem;
    margin: 0;
  }
  dt {
    color: var(--muted);
  }
  dd {
    margin: 0;
  }
  .wrapany {
    overflow-wrap: anywhere;
  }
  kbd {
    font-family: var(--mono);
    border: 1px solid var(--line);
    border-bottom-width: 2px;
    border-radius: 5px;
    padding: 0 0.4em;
    background: var(--bg-3);
  }
  @media (max-width: 560px) {
    .item {
      flex-direction: column;
      align-items: stretch;
    }
  }
</style>
