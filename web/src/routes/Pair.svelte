<script lang="ts">
  import { api } from "../lib/api";

  let { code, onpaired }: { code: string | null; onpaired: () => void } = $props();

  let status = $state<"idle" | "working" | "failed">("idle");
  let message = $state("");

  function deviceName(): string {
    const ua = navigator.userAgent;
    const model = /Android [\d.]+; ([^;)]+)/.exec(ua)?.[1]?.trim();
    if (model) return model;
    if (/iPhone/.test(ua)) return "iPhone";
    if (/iPad/.test(ua)) return "iPad";
    return /Windows/.test(ua) ? "PC Windows" : /Mac/.test(ua) ? "Mac" : "Perangkat";
  }

  async function pair(c: string) {
    status = "working";
    try {
      await api.pair(c, deviceName());
      history.replaceState(null, "", "#/"); // the one-time code must not linger in history
      onpaired();
    } catch (e) {
      status = "failed";
      message = (e as Error).message;
    }
  }

  $effect(() => {
    if (code && status === "idle") void pair(code);
  });
</script>

<div class="card box">
  <h1>Pasangkan perangkat ini</h1>
  {#if status === "working"}
    <p class="muted">Memasangkan...</p>
  {:else if status === "failed"}
    <p class="warn">{message}</p>
    <p class="muted small">Di laptop, buka Pengaturan, Akses dari HP, lalu Buat kode baru dan pindai ulang.</p>
  {:else}
    <p class="muted">
      Perangkat ini belum dipasangkan dengan ChordKu. Di laptop, buka <strong>Pengaturan</strong>, bagian
      <strong>Akses dari HP</strong>, lalu pindai kode QR yang tampil di sana dengan kamera perangkat ini.
    </p>
  {/if}
</div>

<style>
  .box {
    max-width: 34rem;
    margin: 12vh auto 0;
    padding: 1.4rem;
    display: grid;
    gap: 0.7rem;
  }
  h1 {
    font-size: 1.25rem;
  }
  p {
    margin: 0;
  }
  .warn {
    color: var(--bad);
  }
</style>
