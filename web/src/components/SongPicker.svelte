<script lang="ts">
  // Pick songs from the library for a setlist: a searchable list with checkboxes.
  import { api, type Song } from "../lib/api";
  import { untrack } from "svelte";
  import { fail } from "../lib/toast.svelte";

  let { selected, onsave, onclose }: { selected: string[]; onsave: (ids: string[]) => Promise<void> | void; onclose: () => void } = $props();

  let el = $state<HTMLDialogElement>();
  let all = $state<Song[]>([]);
  let query = $state("");
  // eslint-disable-next-line svelte/valid-compile -- the initial selection is all we want here
  let picked = $state(new Set(untrack(() => selected)));
  let busy = $state(false);

  $effect(() => {
    api.songs().then((r) => (all = r.songs), fail);
  });
  $effect(() => {
    if (el && !el.open) el.showModal();
  });

  const shown = $derived.by(() => {
    const q = query.trim().toLowerCase();
    return q ? all.filter((s) => `${s.title} ${s.artist}`.toLowerCase().includes(q)) : all;
  });
  const toggle = (id: string) => {
    const next = new Set(picked);
    if (!next.delete(id)) next.add(id);
    picked = next;
  };

  async function save() {
    busy = true;
    try {
      // Keep the setlist's existing order, append the new picks in library order.
      const kept = selected.filter((id) => picked.has(id));
      const added = all.map((s) => s.id).filter((id) => picked.has(id) && !selected.includes(id));
      await onsave([...kept, ...added]);
      onclose();
    } catch (e) {
      fail(e);
    } finally {
      busy = false;
    }
  }
</script>

<dialog bind:this={el} class="card" oncancel={(e) => (e.preventDefault(), onclose())} onclick={(e) => e.target === el && onclose()}>
  <div class="head row">
    <h3 class="grow">Tambah lagu dari pustaka</h3>
    <button class="ghost icon" onclick={onclose} aria-label="Tutup">✕</button>
  </div>
  <input type="search" placeholder="Cari judul atau artis" bind:value={query} />
  <ul>
    {#each shown as s (s.id)}
      <li>
        <label class="row">
          <input type="checkbox" checked={picked.has(s.id)} onchange={() => toggle(s.id)} />
          <span class="grow">
            <span class="ellipsis">{s.title}</span>
            <span class="muted small ellipsis">{s.artist || "Artis tidak diketahui"}{s.has_chart ? "" : " · belum ada chord"}</span>
          </span>
        </label>
      </li>
    {:else}
      <li class="muted small">{all.length ? "Tidak ada yang cocok." : "Pustaka masih kosong."}</li>
    {/each}
  </ul>
  <div class="row end">
    <span class="muted small grow">{picked.size} lagu dipilih</span>
    <button class="ghost" onclick={onclose}>Batal</button>
    <button class="primary" onclick={save} disabled={busy}>Simpan</button>
  </div>
</dialog>

<style>
  dialog {
    border: 1px solid var(--line);
    padding: 1rem 1.1rem;
    width: min(94vw, 34rem);
    color: inherit;
    display: grid;
    gap: 0.7rem;
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
  input[type="search"] {
    width: 100%;
  }
  ul {
    list-style: none;
    margin: 0;
    padding: 0;
    max-height: 55vh;
    overflow: auto;
    display: grid;
    gap: 0.2rem;
  }
  label {
    gap: 0.6rem;
    padding: 0.4rem 0.3rem;
    border-radius: 8px;
    cursor: pointer;
  }
  label:hover {
    background: var(--bg-3);
  }
  label .grow {
    display: grid;
    min-width: 0;
  }
  .end {
    gap: 0.5rem;
  }
</style>
