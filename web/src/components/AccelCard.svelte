<script lang="ts">
  import { api, onServerEvent, type SystemInfo } from "../lib/api";
  import { bytes } from "../lib/i18n/id";
  import { fail, toast } from "../lib/toast.svelte";

  let { info, onchanged }: { info: SystemInfo; onchanged: () => void } = $props();

  let cudaProgress = $state<number | null>(null);

  const DEVICE: Record<string, string> = {
    cpu: "CPU",
    cuda: "GPU NVIDIA (CUDA)",
    cuda_fp16: "GPU NVIDIA (CUDA, presisi setengah)",
    gpu_high: "GPU NVIDIA (DirectML)",
    gpu_low: "GPU Intel Arc (DirectML)",
    npu: "NPU Intel AI Boost",
  };
  const MODEL: Record<string, string> = { chords: "Chord", beats: "Ketukan", stems: "Pisah stem" };

  $effect(() =>
    onServerEvent((e) => {
      if (e.type !== "model" || e.id !== "cuda") return;
      if (e.done) {
        cudaProgress = null;
        if (e.error) toast(`Unduhan CUDA gagal: ${e.error}`, "error");
        else {
          toast("Pustaka CUDA terpasang. Mulai ulang ChordKu agar dipakai.", "ok");
          onchanged();
        }
      } else cudaProgress = e.progress;
    }),
  );

  async function installCuda() {
    try {
      cudaProgress = 0;
      await api.installCuda();
    } catch (e) {
      cudaProgress = null;
      fail(e);
    }
  }

  async function calibrate() {
    try {
      await api.calibrate();
      toast("Kalibrasi dimulai. Hasilnya muncul di sini dalam setengah menit.", "info");
      onchanged();
    } catch (e) {
      fail(e);
    }
  }

  const cal = $derived(info.accel.calibration);
  const fidelity = (t: { model: string; fidelity: number }) =>
    t.model === "stems" ? `${t.fidelity >= 199 ? "identik" : t.fidelity.toFixed(0) + " dB"}` : `${(t.fidelity * 100).toFixed(1)} %`;
</script>

<section class="card">
  <div class="row wrap">
    <h2 class="grow">Akselerasi perangkat keras</h2>
    <button onclick={calibrate} disabled={info.accel.calibrating}>{info.accel.calibrating ? "Mengkalibrasi..." : "Kalibrasi ulang"}</button>
  </div>
  <p class="muted small">
    ChordKu mengukur tiap model di tiap prosesor laptop ini, lalu hanya memakai perangkat yang terbukti lebih cepat dan hasilnya sama
    dengan CPU. Perangkat yang cepat tetapi hasilnya menyimpang ditolak. Untuk pemisah stem di GPU NVIDIA juga dicoba salinan model
    berpresisi setengah (dibuat otomatis di laptop ini): memori GPU separuhnya dan lebih cepat, dipakai bila selisihnya dari CPU di
    bawah ambang dengar (minimal 40 dB).
  </p>

  {#if cal}
    <div class="plan row wrap">
      {#each ["chords", "beats", "stems"] as m}
        {#if m === "stems" && !cal.stems_measured}
          <span class="pill big">{MODEL[m]}: <strong>belum diukur</strong>&nbsp;(unduh modelnya dulu)</span>
        {:else}
          <span class="pill big">{MODEL[m]}: <strong>{DEVICE[cal.plan[m as "chords"]] ?? cal.plan[m as "chords"]}</strong></span>
        {/if}
      {/each}
    </div>
    <details class="small">
      <summary>Hasil pengukuran</summary>
      <table>
        <thead><tr><th>Model</th><th>Perangkat</th><th>Waktu</th><th>Sama dengan CPU</th><th></th></tr></thead>
        <tbody>
          {#each cal.trials as t}
            <tr class:chosen={t.accepted}>
              <td>{MODEL[t.model] ?? t.model}</td>
              <td>{DEVICE[t.device] ?? t.device}</td>
              <td class="mono">{t.available ? `${t.run_s.toFixed(2)} s` : "-"}</td>
              <td class="mono">{t.available ? fidelity(t) : "-"}</td>
              <td class="muted">{t.accepted ? "dipakai" : t.available ? t.note : "tidak tersedia"}</td>
            </tr>
          {/each}
        </tbody>
      </table>
    </details>
  {:else if info.accel.calibrating}
    <p class="small">Sedang mengukur perangkat. Lagu pertama mungkin menunggu beberapa detik.</p>
  {:else}
    <p class="muted small">Belum dikalibrasi (model belum lengkap).</p>
  {/if}

  {#if info.cuda.supported && info.cuda.nvidia_gpu && !info.cuda.installed}
    <div class="item">
      <div class="grow">
        <strong>Pustaka NVIDIA CUDA</strong>
        <div class="muted small">
          Diperlukan agar pemisah stem berjalan di GPU NVIDIA (sekitar 5 kali lebih cepat). Diunduh dari paket resmi NVIDIA di PyPI
          dengan checksum terverifikasi.
        </div>
        {#if cudaProgress !== null}<div class="bar"><i style="width:{Math.round(cudaProgress * 100)}%"></i></div>{/if}
      </div>
      <button class="primary" onclick={installCuda} disabled={cudaProgress !== null}>
        {cudaProgress !== null ? `${Math.round(cudaProgress * 100)}%` : `Unduh (${bytes(info.cuda.download_bytes)})`}
      </button>
    </div>
  {:else if info.cuda.installed}
    <p class="muted small">Pustaka NVIDIA CUDA terpasang di <span class="mono">{info.cuda.dir}</span></p>
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
  .plan {
    gap: 0.4rem;
  }
  .pill.big {
    font-size: 0.9rem;
    padding: 0.25em 0.75em;
    color: var(--text);
  }
  table {
    width: 100%;
    border-collapse: collapse;
    margin-top: 0.5rem;
  }
  th,
  td {
    text-align: left;
    padding: 0.25rem 0.5rem 0.25rem 0;
    border-bottom: 1px solid var(--line);
    vertical-align: top;
  }
  tr.chosen td {
    color: var(--good);
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
</style>
