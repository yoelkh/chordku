<script lang="ts">
  import Library from "./routes/Library.svelte";
  import Settings from "./routes/Settings.svelte";
  import SongView from "./routes/Song.svelte";
  import Pair from "./routes/Pair.svelte";
  import Dialog from "./components/Dialog.svelte";
  import { api } from "./lib/api";
  import { dismiss, toasts } from "./lib/toast.svelte";

  // Hash routing: "#/", "#/lagu/<id>", "#/pengaturan". No server round trips, works from a file too.
  let hash = $state(location.hash);
  $effect(() => {
    const onHash = () => (hash = location.hash);
    addEventListener("hashchange", onHash);
    return () => removeEventListener("hashchange", onHash);
  });

  // On the laptop (loopback) everything is open. From the LAN the device must be paired first.
  let session = $state<{ lan: boolean; paired: boolean } | null>(null);
  const checkSession = () =>
    api
      .session()
      .then((s) => (session = s))
      .catch(() => (session = { lan: false, paired: true }));
  void checkSession();

  const route = $derived.by(() => {
    const parts = hash.replace(/^#\/?/, "").split("/");
    if (parts[0] === "pair") return { name: "pair" as const, code: parts[1] ?? null };
    if (session && !session.paired) return { name: "pair" as const, code: null };
    if (parts[0] === "lagu" && parts[1]) return { name: "song" as const, id: parts[1] };
    if (parts[0] === "pengaturan") return { name: "settings" as const };
    return { name: "library" as const };
  });
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
    <span>ChordKu</span>
  </a>
  <nav>
    <a href="#/" class:active={route.name !== "settings"}>Pustaka</a>
    <a href="#/pengaturan" class:active={route.name === "settings"}>Pengaturan</a>
  </nav>
</header>

<main>
  {#if !session}
    <p class="muted">Memuat...</p>
  {:else if route.name === "pair"}
    <Pair code={route.code} onpaired={() => checkSession().then(() => (hash = location.hash))} />
  {:else if route.name === "song"}
    {#key route.id}
      <SongView id={route.id} />
    {/key}
  {:else if route.name === "settings"}
    <Settings />
  {:else}
    <Library />
  {/if}
</main>

<Dialog />

<div class="toasts no-print" aria-live="polite">
  {#each toasts as t (t.id)}
    <button class="toast {t.kind}" onclick={() => dismiss(t.id)}>{t.text}</button>
  {/each}
</div>

<style>
  .top {
    position: sticky;
    top: 0;
    z-index: 20;
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 0.55rem clamp(0.8rem, 3vw, 1.6rem);
    background: color-mix(in srgb, var(--bg) 88%, transparent);
    backdrop-filter: blur(10px);
    border-bottom: 1px solid var(--line);
  }
  .brand {
    display: flex;
    align-items: center;
    gap: 0.55rem;
    color: var(--text);
    font-weight: 700;
    font-size: 1.08rem;
  }
  nav {
    display: flex;
    gap: 0.25rem;
  }
  nav a {
    color: var(--muted);
    padding: 0.35rem 0.75rem;
    border-radius: 8px;
  }
  nav a.active {
    color: var(--text);
    background: var(--bg-3);
  }
  main {
    max-width: 1180px;
    margin: 0 auto;
    padding: clamp(0.8rem, 2.5vw, 1.6rem);
    padding-bottom: 9rem;
  }
  .toasts {
    position: fixed;
    right: 1rem;
    bottom: 1rem;
    z-index: 60;
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
    max-width: min(28rem, calc(100vw - 2rem));
  }
  .toast {
    text-align: left;
    background: var(--bg-2);
    border-left: 4px solid var(--info);
    box-shadow: 0 8px 30px rgb(0 0 0 / 0.35);
  }
  .toast.error {
    border-left-color: var(--bad);
  }
  .toast.ok {
    border-left-color: var(--good);
  }
</style>
