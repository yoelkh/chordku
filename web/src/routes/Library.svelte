<script lang="ts">
  import AddSong from "../components/AddSong.svelte";
  import { api, onServerEvent, type Job, type Setlist, type Song } from "../lib/api";
  import { clock, jobLabel } from "../lib/i18n/id";
  import { keyName } from "../lib/theory";
  import { fail, toast } from "../lib/toast.svelte";

  let songs = $state<Song[]>([]);
  let tags = $state<string[]>([]);
  let jobs = $state<Record<string, Job>>({}); // latest job per song
  let query = $state("");
  let tag = $state("");
  let favOnly = $state(false);
  let loaded = $state(false);

  // Setlists: when one is selected the list shows its songs in set order.
  let setlists = $state<Setlist[]>([]);
  let setlistId = $state(0);

  let refreshTimer = 0;
  async function refresh() {
    try {
      setlists = (await api.setlists()).setlists;
      if (setlistId && !setlists.some((l) => l.id === setlistId)) setlistId = 0;
      if (setlistId) {
        songs = (await api.setlist(setlistId)).songs;
      } else {
        const res = await api.songs(query, tag, favOnly);
        songs = res.songs;
        tags = res.tags;
      }
    } catch (e) {
      fail(e);
    } finally {
      loaded = true;
    }
  }
  function refreshSoon(ms = 150) {
    clearTimeout(refreshTimer);
    refreshTimer = window.setTimeout(refresh, ms);
  }

  $effect(() => {
    // Re-run on any filter change.
    void query, tag, favOnly, setlistId;
    refreshSoon(query ? 250 : 0);
  });

  $effect(() =>
    onServerEvent((e) => {
      if (e.type === "job") {
        jobs[e.job.song_id] = e.job;
        if (e.job.state === "failed" && Date.now() / 1000 - e.job.updated_at < 5) toast(jobLabel(e.job), "error");
      } else {
        refreshSoon();
      }
    }),
  );

  function onadded(id: string, existing: boolean) {
    if (existing) location.hash = `#/lagu/${id}`;
    else refreshSoon(0);
  }

  async function toggleFav(song: Song, e: Event) {
    e.preventDefault();
    e.stopPropagation();
    try {
      await api.patchSong(song.id, { fav: !song.fav });
      song.fav = !song.fav;
    } catch (err) {
      fail(err);
    }
  }

  async function newSetlist() {
    const name = prompt("Nama setlist baru (misalnya Latihan Sabtu):")?.trim();
    if (!name) return;
    try {
      const res = await api.createSetlist(name);
      setlists = res.setlists;
      setlistId = res.id;
    } catch (e) {
      fail(e);
    }
  }

  async function renameSetlist() {
    const current = setlists.find((l) => l.id === setlistId);
    const name = prompt("Nama setlist:", current?.name)?.trim();
    if (name) await api.renameSetlist(setlistId, name).then((r) => (setlists = r.setlists), fail);
  }

  async function deleteSetlist() {
    if (!confirm("Hapus setlist ini? Lagu-lagunya tetap ada di pustaka.")) return;
    await api.deleteSetlist(setlistId).then((r) => ((setlists = r.setlists), (setlistId = 0)), fail);
  }

  /** Move a song within the setlist (`delta` = -1 / +1) or drop it (`delta` = 0). */
  async function reorder(index: number, delta: number, e: Event) {
    e.preventDefault();
    e.stopPropagation();
    const ids = songs.map((s) => s.id);
    if (delta === 0) ids.splice(index, 1);
    else {
      const to = index + delta;
      if (to < 0 || to >= ids.length) return;
      [ids[index], ids[to]] = [ids[to], ids[index]];
    }
    await api.orderSetlist(setlistId, ids).then((r) => (songs = r.songs), fail);
  }

  const thumbCss =(url: string | null) => (url ? "url('" + encodeURI(url).replace(/'/g, "%27") + "')" : undefined);
  const active = (j?: Job) => j && (j.state === "queued" || j.state === "running");
</script>

<AddSong {onadded} />

<div class="filters row wrap">
  <select bind:value={setlistId} aria-label="Pustaka atau setlist">
    <option value={0}>Semua lagu</option>
    {#each setlists as l (l.id)}<option value={l.id}>Setlist: {l.name} ({l.count})</option>{/each}
  </select>
  {#if setlistId}
    <button class="small" onclick={renameSetlist}>Ganti nama</button>
    <button class="small danger" onclick={deleteSetlist}>Hapus setlist</button>
  {:else}
    <button class="small" onclick={newSetlist}>Setlist baru</button>
  {/if}
</div>

<div class="filters row wrap" class:hidden={!!setlistId}>
  <input class="grow" type="search" placeholder="Cari judul atau artis" bind:value={query} />
  {#if tags.length}
    <select bind:value={tag} aria-label="Saring menurut label">
      <option value="">Semua label</option>
      {#each tags as t}<option value={t}>{t}</option>{/each}
    </select>
  {/if}
  <button class:on={favOnly} onclick={() => (favOnly = !favOnly)} aria-pressed={favOnly}>★ Favorit</button>
</div>

{#if loaded && !songs.length}
  <div class="empty muted">
    {#if query || tag || favOnly}
      Tidak ada lagu yang cocok.
    {:else}
      <strong>Pustaka masih kosong.</strong>
      <span>Tempel tautan lagu di atas, pilih file, atau rekam. Chord muncul dalam beberapa detik.</span>
    {/if}
  </div>
{/if}

<ul class="songs">
  {#each songs as song, index (song.id)}
    {@const job = jobs[song.id]}
    <li>
      <a class="card song" href="#/lagu/{song.id}">
        <div class="thumb" style:background-image={thumbCss(song.thumbnail)}>
          {#if !song.thumbnail}<span>{song.src_kind === "mic" ? "●" : "♪"}</span>{/if}
        </div>
        <div class="grow info">
          <div class="title ellipsis">{song.title}</div>
          <div class="muted small ellipsis">{song.artist || (song.src_kind === "mic" ? "Rekaman mikrofon" : "Artis tidak diketahui")}</div>
          <div class="meta row wrap">
            {#if active(job)}
              <span class="pill working">{jobLabel(job!)}{job!.state === "running" && job!.progress > 0 ? ` ${Math.round(job!.progress * 100)}%` : ""}</span>
            {:else if job?.state === "failed" && !song.has_chart}
              <span class="pill failed" title={job.error ?? ""}>Gagal</span>
            {/if}
            {#if song.key}<span class="pill">Kunci {keyName(song.key)}</span>{/if}
            {#if song.bpm}<span class="pill">{Math.round(song.bpm)} BPM</span>{/if}
            {#if song.duration_ms}<span class="pill">{clock(song.duration_ms)}</span>{/if}
            {#if song.has_stems}<span class="pill">Stem</span>{/if}
            {#each song.tags as t}<span class="pill tag">{t}</span>{/each}
          </div>
        </div>
        {#if setlistId}
          <span class="order">
            <button class="ghost icon small" onclick={(e) => reorder(index, -1, e)} disabled={index === 0} aria-label="Naikkan">↑</button>
            <button class="ghost icon small" onclick={(e) => reorder(index, 1, e)} disabled={index === songs.length - 1} aria-label="Turunkan">↓</button>
            <button class="ghost icon small danger" onclick={(e) => reorder(index, 0, e)} aria-label="Keluarkan dari setlist">✕</button>
          </span>
        {/if}
        <button class="ghost icon fav" class:faved={song.fav} onclick={(e) => toggleFav(song, e)} aria-label={song.fav ? "Hapus dari favorit" : "Jadikan favorit"}>★</button>
      </a>
    </li>
  {/each}
</ul>

<style>
  .filters {
    margin: 1rem 0 0.8rem;
  }
  .filters + .filters {
    margin-top: 0;
  }
  .hidden {
    display: none;
  }
  .order {
    display: flex;
    flex: none;
  }
  .empty {
    display: grid;
    gap: 0.3rem;
    text-align: center;
    padding: 3rem 1rem;
  }
  .songs {
    list-style: none;
    margin: 0;
    padding: 0;
    display: grid;
    gap: 0.55rem;
    grid-template-columns: repeat(auto-fill, minmax(min(100%, 25rem), 1fr));
  }
  .song {
    display: flex;
    align-items: center;
    gap: 0.8rem;
    padding: 0.6rem;
    color: inherit;
    transition: border-color 0.12s;
  }
  .song:hover {
    border-color: var(--muted);
  }
  .thumb {
    flex: none;
    width: 4.6rem;
    aspect-ratio: 16 / 10;
    border-radius: 7px;
    background: var(--bg-3) center / cover no-repeat;
    display: grid;
    place-items: center;
    color: var(--muted);
    font-size: 1.4rem;
  }
  .info {
    display: grid;
    gap: 0.1rem;
  }
  .title {
    font-weight: 600;
  }
  .meta {
    gap: 0.3rem;
    margin-top: 0.25rem;
  }
  .pill.working {
    color: var(--accent);
    border-color: var(--accent);
  }
  .pill.failed {
    color: var(--bad);
    border-color: var(--bad);
  }
  .pill.tag {
    color: var(--info);
  }
  .fav {
    color: var(--line);
    font-size: 1.2rem;
  }
  .fav.faved {
    color: var(--accent);
  }
</style>
