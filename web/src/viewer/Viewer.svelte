<script lang="ts">
  // Root of the standalone viewer: the setlist page and one song page at a time.
  import SongPage from "./SongPage.svelte";
  import { loadData, type ViewerData } from "./data";
  import { clock } from "../lib/i18n/id";
  import { keyName } from "../lib/theory";

  let data = $state<ViewerData | null>(null);
  let error = $state("");
  loadData().then(
    (d) => (d ? (data = d) : (error = "File ini tidak berisi setlist. Unduh lagi dari ChordKu (Pustaka > setlist > Bagikan).")),
    (e: Error) => (error = e.message),
  );

  // Hash routing: "#/" the list, "#/3" the fourth song. Survives reloads and works from a file.
  let hash = $state(location.hash);
  $effect(() => {
    const onHash = () => (hash = location.hash);
    addEventListener("hashchange", onHash);
    return () => removeEventListener("hashchange", onHash);
  });
  const index = $derived.by(() => {
    const part = hash.replace(/^#\/?/, "");
    const n = part === "" ? -1 : Number(part);
    return data && Number.isInteger(n) && n >= 0 && n < data.songs.length ? n : -1;
  });
  const go = (i: number) => (location.hash = i < 0 ? "#/" : `#/${i}`);

  $effect(() => {
    if (data) document.title = index >= 0 ? `${data.songs[index].title} · ${data.setlist.name}` : `${data.setlist.name} · ChordKu`;
  });
  const date = (s: number) => new Date(s * 1000).toLocaleDateString("id-ID", { day: "numeric", month: "long", year: "numeric" });
</script>

<header class="top no-print">
  <a class="brand" href="#/">
    <svg viewBox="0 0 64 64" width="26" height="26" aria-hidden="true">
      <rect width="64" height="64" rx="14" fill="var(--bg-3)" />
      <g stroke="var(--accent)" stroke-width="3" stroke-linecap="round" opacity=".45">
        <path d="M18 14v36M28 14v36M38 14v36M48 14v36M14 24h38M14 36h38" />
      </g>
      <circle cx="28" cy="30" r="5" fill="var(--accent)" /><circle cx="38" cy="42" r="5" fill="var(--accent)" /><circle cx="48" cy="30" r="5" fill="var(--accent)" />
    </svg>
    <span>{data?.setlist.name ?? "ChordKu"}</span>
  </a>
  {#if data && index >= 0}
    <nav>
      <button class="ghost small" onclick={() => go(index - 1)} disabled={index === 0} aria-label="Lagu sebelumnya">‹</button>
      <span class="muted small mono">{index + 1}/{data.songs.length}</span>
      <button class="ghost small" onclick={() => go(index + 1)} disabled={index === data.songs.length - 1} aria-label="Lagu berikutnya">›</button>
    </nav>
  {/if}
</header>

<main>
  {#if error}
    <p class="card err">{error}</p>
  {:else if !data}
    <p class="muted">Memuat...</p>
  {:else if index >= 0}
    {#key index}
      <SongPage song={data.songs[index]} setlist={data.setlist} {index} total={data.songs.length} onnav={go} />
    {/key}
  {:else}
    <h1>{data.setlist.name}</h1>
    <p class="muted small">{data.songs.length} lagu · diekspor {date(data.exported_at)} dari ChordKu. Tiap orang bisa mengatur transpose dan capo sendiri di HP-nya.</p>
    <ol class="songs">
      {#each data.songs as s, i (s.id)}
        <li>
          <a class="card song" href="#/{i}">
            <span class="num mono">{i + 1}</span>
            <span class="grow info">
              <span class="title ellipsis">{s.title}</span>
              <span class="muted small ellipsis">{s.artist || "Artis tidak diketahui"}</span>
            </span>
            <span class="meta row">
              {#if s.analysis.key}<span class="pill">Kunci {keyName(s.analysis.key)}</span>{/if}
              {#if s.analysis.bpm}<span class="pill">{Math.round(s.analysis.bpm)} BPM</span>{/if}
              <span class="pill">{clock(s.duration_ms || s.analysis.duration_ms)}</span>
            </span>
          </a>
        </li>
      {/each}
    </ol>
    <p class="muted small foot">Dibuat dengan <a href="https://github.com/yoelkh/chordku">ChordKu</a>. Halaman ini hanya berisi chord, tanpa audio.</p>
  {/if}
</main>

<style>
  .top {
    position: sticky;
    top: 0;
    z-index: 20;
    display: flex;
    align-items: center;
    gap: 1rem;
    padding: 0.55rem 1rem;
    background: color-mix(in srgb, var(--bg) 88%, transparent);
    backdrop-filter: blur(10px);
    border-bottom: 1px solid var(--line);
  }
  .brand {
    display: flex;
    align-items: center;
    gap: 0.55rem;
    font-weight: 700;
    color: inherit;
    text-decoration: none;
    min-width: 0;
  }
  .brand span {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  nav {
    margin-left: auto;
    display: flex;
    align-items: center;
    gap: 0.3rem;
    flex: none;
  }
  main {
    max-width: 1100px;
    margin: 0 auto;
    padding: 0.8rem 1rem 6rem;
  }
  h1 {
    margin: 0.6rem 0 0.2rem;
  }
  .songs {
    list-style: none;
    margin: 1rem 0 0;
    padding: 0;
    display: grid;
    gap: 0.55rem;
  }
  .song {
    display: flex;
    align-items: center;
    gap: 0.8rem;
    padding: 0.7rem 0.9rem;
    color: inherit;
    text-decoration: none;
  }
  .song:hover {
    border-color: var(--muted);
  }
  .num {
    color: var(--muted);
    min-width: 1.6rem;
    text-align: right;
  }
  .info {
    display: grid;
    min-width: 0;
  }
  .title {
    font-weight: 600;
  }
  .meta {
    gap: 0.3rem;
    flex: none;
    flex-wrap: wrap;
    justify-content: flex-end;
  }
  .err {
    padding: 1rem;
    border-color: var(--bad);
  }
  .foot {
    margin-top: 2rem;
    text-align: center;
  }
  @media (max-width: 640px) {
    .meta {
      display: none;
    }
  }
</style>
