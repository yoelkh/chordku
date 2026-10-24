<script lang="ts">
  import AddSong from "../components/AddSong.svelte";
  import SongPicker from "../components/SongPicker.svelte";
  import { api, onServerEvent, type Job, type Setlist, type Song } from "../lib/api";
  import { clock, jobLabel } from "../lib/i18n/id";
  import { keyName } from "../lib/theory";
  import { askConfirm, askText } from "../lib/dialog.svelte";
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

  const setlistName = () => setlists.find((l) => l.id === setlistId)?.name ?? "";

  /** A song added while a setlist is open goes into that setlist, new or already known. */
  async function onadded(id: string, existing: boolean) {
    if (setlistId) {
      try {
        setlists = (await api.addToSetlist(setlistId, id)).setlists;
        toast(`Ditambahkan ke setlist ${setlistName()}.`, "ok");
      } catch (e) {
        fail(e);
      }
      refreshSoon(0);
    } else if (existing) location.hash = `#/lagu/${id}`;
    else refreshSoon(0);
  }

  // "Tambah dari pustaka" for the open setlist.
  let picking = $state(false);
  async function savePicked(ids: string[]) {
    songs = (await api.orderSetlist(setlistId, ids)).songs;
    setlists = (await api.setlists()).setlists;
  }

  let shareMenu = $state(false);

  // "+ setlist" on a library row: a small menu of setlists.
  let rowMenu = $state<string | null>(null);
  async function addRowTo(songId: string, value: string) {
    rowMenu = null;
    try {
      let target = Number(value);
      if (value === "new") {
        const name = await askText("Nama setlist baru", "", { placeholder: "misalnya Ibadah Minggu" });
        if (!name) return;
        target = (await api.createSetlist(name)).id;
      }
      setlists = (await api.addToSetlist(target, songId)).setlists;
      toast(`Ditambahkan ke setlist ${setlists.find((l) => l.id === target)?.name ?? ""}.`, "ok");
    } catch (e) {
      fail(e);
    }
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
    const name = await askText("Nama setlist baru", "", { placeholder: "misalnya Ibadah Minggu" });
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
    const name = await askText("Nama setlist", current?.name ?? "");
    if (name) await api.renameSetlist(setlistId, name).then((r) => (setlists = r.setlists), fail);
  }

  async function deleteSetlist() {
    if (!(await askConfirm("Hapus setlist ini?", { text: "Lagu-lagunya tetap ada di pustaka.", ok: "Hapus", danger: true }))) return;
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

<AddSong {onadded} hint={setlistId ? `Lagu yang ditambahkan di sini langsung masuk ke setlist "${setlistName()}".` : ""} />

<div class="filters row wrap">
  <select bind:value={setlistId} aria-label="Pustaka atau setlist">
    <option value={0}>Semua lagu</option>
    {#each setlists as l (l.id)}<option value={l.id}>Setlist: {l.name} ({l.count})</option>{/each}
  </select>
  {#if setlistId}
    <button class="small primary" onclick={() => (picking = true)}>Tambah dari pustaka</button>
    <span class="menu-wrap">
      <button class="small" class:on={shareMenu} onclick={() => (shareMenu = !shareMenu)} aria-haspopup="menu" aria-expanded={shareMenu}>Bagikan dengan audio</button>
      {#if shareMenu}
        <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
        <div class="card pop share" role="menu" tabindex="-1" onclick={() => (shareMenu = false)}>
          <div class="muted small">Satu file HTML berisi chord dan audio semua lagu. Lagu yang belum dipisahkan memakai audio aslinya.</div>
          <a class="option" href={api.shareSetlistUrl(setlistId, "stems")} download>
            <strong>4 instrumen</strong><span class="muted small">vokal, drum, bass, gitar/piano: bisa dibisukan atau di-solo. Paling besar; paling enak di laptop/tablet.</span>
          </a>
          <a class="option" href={api.shareSetlistUrl(setlistId, "light")} download>
            <strong>Vokal + iringan</strong><span class="muted small">dua jalur ringan, cocok untuk HP: bisukan vokal untuk latihan.</span>
          </a>
          <a class="option" href={api.shareSetlistUrl(setlistId, "mix")} download>
            <strong>Lagu asli</strong><span class="muted small">tanpa pemisahan.</span>
          </a>
        </div>
      {/if}
    </span>
    <a class="button small" href={api.shareSetlistUrl(setlistId)} download title="Satu file HTML kecil berisi chord saja (tanpa audio); lagu dari YouTube diputar lewat video tersemat.">Bagikan chord saja</a>
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
    {#if setlistId}
      <strong>Setlist masih kosong.</strong>
      <span>Tekan "Tambah dari pustaka", atau tempel tautan / pilih file di atas: lagunya langsung masuk ke setlist ini.</span>
    {:else if query || tag || favOnly}
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
        {:else}
          <span class="order menu-wrap">
            <button class="ghost icon small" title="Tambahkan ke setlist" aria-label="Tambahkan ke setlist" onclick={(e) => (e.preventDefault(), e.stopPropagation(), (rowMenu = rowMenu === song.id ? null : song.id))}>＋</button>
            {#if rowMenu === song.id}
              <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
              <div class="card pop" role="menu" tabindex="-1" onclick={(e) => (e.preventDefault(), e.stopPropagation())}>
                <div class="muted small">Tambahkan ke setlist</div>
                {#each setlists as l (l.id)}<button class="option" onclick={() => addRowTo(song.id, String(l.id))}>{l.name}</button>{/each}
                <button class="option" onclick={() => addRowTo(song.id, "new")}>Setlist baru...</button>
              </div>
            {/if}
          </span>
        {/if}
        <button class="ghost icon fav" class:faved={song.fav} onclick={(e) => toggleFav(song, e)} aria-label={song.fav ? "Hapus dari favorit" : "Jadikan favorit"}>★</button>
      </a>
    </li>
  {/each}
</ul>

{#if picking}
  <SongPicker selected={songs.map((s) => s.id)} onsave={savePicked} onclose={() => (picking = false)} />
{/if}

<style>
  .filters {
    margin: 1rem 0 0.8rem;
  }
  a.button {
    display: inline-block;
    border: 1px solid var(--line);
    background: var(--bg-3);
    border-radius: 8px;
    padding: 0.45em 0.85em;
    color: inherit;
    text-decoration: none;
  }
  a.button:hover {
    border-color: var(--muted);
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
  .menu-wrap {
    position: relative;
  }
  .pop {
    position: absolute;
    right: 0;
    top: 2.2rem;
    z-index: 30;
    min-width: 12rem;
    padding: 0.4rem;
    display: grid;
    gap: 0.15rem;
    box-shadow: 0 12px 40px rgb(0 0 0 / 0.45);
  }
  .pop .option {
    text-align: left;
    background: transparent;
    border-color: transparent;
  }
  .pop.share {
    left: 0;
    right: auto;
    width: min(22rem, calc(100vw - 2rem));
  }
  .pop.share > .muted {
    padding: 0.2rem 0.4rem 0.3rem;
  }
  a.option {
    display: grid;
    gap: 0.1rem;
    padding: 0.45rem 0.5rem;
    border-radius: 8px;
    color: inherit;
    text-decoration: none;
  }
  a.option:hover {
    background: var(--bg-3);
  }
  .pop .option:hover {
    background: var(--bg-3);
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
