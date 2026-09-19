<script lang="ts">
  import { api, onServerEvent, type LanState } from "../lib/api";
  import { fail } from "../lib/toast.svelte";

  // (Not named `state`: `$state` would then read as a store subscription.)
  let lan = $state<LanState | null>(null);
  let busy = $state(false);

  async function refresh() {
    try {
      lan = await api.lan();
    } catch (e) {
      fail(e);
    }
  }
  void refresh();
  $effect(() => onServerEvent((e) => e.type === "system" && void refresh()));

  async function run(fn: () => Promise<LanState>) {
    busy = true;
    try {
      lan = await fn();
    } catch (e) {
      fail(e);
      await refresh();
    } finally {
      busy = false;
    }
  }

  const ago = (ts: number) => {
    const mins = Math.max(0, Math.round((Date.now() / 1000 - ts) / 60));
    return mins < 2 ? "baru saja" : mins < 120 ? `${mins} menit lalu` : mins < 2880 ? `${Math.round(mins / 60)} jam lalu` : `${Math.round(mins / 1440)} hari lalu`;
  };
</script>

<section class="card">
  <div class="row wrap">
    <h2 class="grow">Akses dari HP (mode LAN aman)</h2>
    {#if lan}
      <button class:primary={!lan.lan.enabled} disabled={busy} onclick={() => run(() => api.setLan(!lan!.lan.enabled))}>
        {lan.lan.enabled ? "Matikan" : "Aktifkan"}
      </button>
    {/if}
  </div>
  <p class="muted small">
    Membuka ChordKu untuk HP di Wi-Fi yang sama lewat HTTPS, supaya perekam mikrofon di HP bisa dipakai. Hanya perangkat yang
    dipasangkan lewat kode QR yang bisa masuk. Saat mati, ChordKu hanya bisa dibuka dari laptop ini.
  </p>

  {#if lan?.lan.error}<p class="warn small">{lan.lan.error}</p>{/if}

  {#if lan?.lan.enabled && lan.lan.running}
    <div class="pairing">
      {#if lan.lan.pair_qr_svg}
        <div class="qr" aria-label="Kode QR pemasangan">{@html lan.lan.pair_qr_svg}</div>
      {/if}
      <ol class="small">
        <li>Sambungkan HP ke Wi-Fi yang sama dengan laptop.</li>
        <li>Pindai kode QR ini dengan kamera HP{#if lan.lan.pair_expires_secs} (berlaku {Math.ceil(lan.lan.pair_expires_secs / 60)} menit, sekali pakai){/if}.</li>
        <li>Browser akan memperingatkan soal sertifikat karena dibuat oleh laptop ini sendiri. Pilih <em>Lanjutan</em> lalu <em>Lanjutkan</em>.</li>
        <li>Jika Windows menanyakan izin firewall untuk ChordKu, izinkan untuk jaringan <em>Pribadi</em>.</li>
      </ol>
    </div>
    <div class="row wrap small">
      <button disabled={busy} onclick={() => run(api.newPairCode)}>Buat kode baru</button>
      <span class="muted">Alamat: {#each lan.lan.addresses as a}<span class="mono">{a}</span>{" "}{/each}</span>
    </div>
    <details class="small">
      <summary>Menghilangkan peringatan sertifikat (opsional)</summary>
      <p class="muted">
        Buka <span class="mono">{lan.lan.addresses[0]}/ca.crt</span> di HP untuk mengunduh sertifikat CA lokal, lalu pasang lewat
        Setelan, Keamanan, Enkripsi dan kredensial, Instal sertifikat, Sertifikat CA. Kunci CA ini hanya ada di laptop Anda.
      </p>
    </details>
  {/if}

  {#if lan?.devices.length}
    <div class="devices">
      <div class="muted small">Perangkat terpasang</div>
      {#each lan.devices as d (d.id)}
        <div class="row device">
          <span class="grow ellipsis">{d.name}</span>
          <span class="muted small">aktif {ago(d.last_seen)}</span>
          <button class="small danger" disabled={busy} onclick={() => run(() => api.deleteDevice(d.id))}>Cabut</button>
        </div>
      {/each}
    </div>
  {/if}
</section>

<style>
  section {
    padding: 1rem;
    margin-bottom: 0.9rem;
    display: grid;
    gap: 0.6rem;
  }
  h2 {
    font-size: 1.05rem;
  }
  p {
    margin: 0;
  }
  .warn {
    color: var(--bad);
  }
  .pairing {
    display: flex;
    gap: 1rem;
    align-items: center;
    flex-wrap: wrap;
  }
  .qr {
    background: #fff;
    border-radius: 8px;
    padding: 4px;
    width: 190px;
    flex: none;
  }
  .qr :global(svg) {
    display: block;
    width: 100%;
    height: auto;
  }
  ol {
    margin: 0;
    padding-left: 1.2rem;
    display: grid;
    gap: 0.3rem;
    flex: 1 1 16rem;
  }
  .devices {
    display: grid;
    gap: 0.3rem;
    border-top: 1px solid var(--line);
    padding-top: 0.6rem;
  }
  .device {
    gap: 0.7rem;
  }
  details p {
    margin-top: 0.4rem;
  }
</style>
