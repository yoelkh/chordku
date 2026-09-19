<script lang="ts">
  import { clock } from "../lib/i18n/id";
  import { fail } from "../lib/toast.svelte";

  let { onrecorded, disabled = false }: { onrecorded: (blob: Blob, title: string) => void; disabled?: boolean } = $props();

  // getUserMedia exists only in a secure context (https or localhost). Over plain http on the
  // LAN we fall back to the phone's own recorder through a capture file input.
  const canRecord = typeof navigator !== "undefined" && !!navigator.mediaDevices?.getUserMedia && typeof MediaRecorder !== "undefined";

  let recording = $state(false);
  let elapsed = $state(0);
  let level = $state(0);
  let recorder: MediaRecorder | null = null;
  let stream: MediaStream | null = null;
  let chunks: Blob[] = [];
  let timer = 0;
  let meterCtx: AudioContext | null = null;
  let captureInput = $state<HTMLInputElement>();

  function pickMime(): string | undefined {
    return ["audio/webm;codecs=opus", "audio/ogg;codecs=opus", "audio/mp4"].find((m) => MediaRecorder.isTypeSupported(m));
  }

  async function start() {
    try {
      // Music, not speech: the phone's echo canceller, noise suppressor and AGC all mangle chords.
      stream = await navigator.mediaDevices.getUserMedia({
        audio: { echoCancellation: false, noiseSuppression: false, autoGainControl: false, channelCount: 1 },
      });
    } catch {
      fail(new Error("Mikrofon tidak bisa diakses. Izinkan akses mikrofon untuk halaman ini."));
      return;
    }
    chunks = [];
    recorder = new MediaRecorder(stream, { mimeType: pickMime(), audioBitsPerSecond: 128_000 });
    recorder.ondataavailable = (e) => e.data.size && chunks.push(e.data);
    recorder.onstop = finish;
    recorder.start(1000);
    recording = true;
    const began = performance.now();
    timer = window.setInterval(() => (elapsed = performance.now() - began), 250);

    meterCtx = new AudioContext();
    const analyser = meterCtx.createAnalyser();
    analyser.fftSize = 1024;
    meterCtx.createMediaStreamSource(stream).connect(analyser);
    const data = new Float32Array(analyser.fftSize);
    const tick = () => {
      if (!recording) return;
      analyser.getFloatTimeDomainData(data);
      let peak = 0;
      for (const v of data) peak = Math.max(peak, Math.abs(v));
      level = peak;
      requestAnimationFrame(tick);
    };
    tick();
  }

  function stop() {
    recorder?.stop();
  }

  function finish() {
    clearInterval(timer);
    stream?.getTracks().forEach((t) => t.stop());
    void meterCtx?.close();
    const blob = new Blob(chunks, { type: recorder?.mimeType || "audio/webm" });
    const seconds = elapsed / 1000;
    recording = false;
    elapsed = 0;
    level = 0;
    if (seconds < 5) {
      fail(new Error("Rekaman terlalu pendek. Rekam minimal 5 detik."));
      return;
    }
    const stamp = new Date().toLocaleString("id-ID", { dateStyle: "medium", timeStyle: "short" });
    onrecorded(blob, `Rekaman ${stamp}`);
  }

  function fromCapture(files: FileList | null) {
    const file = files?.[0];
    if (file) onrecorded(file, file.name.replace(/\.[^.]+$/, ""));
    if (captureInput) captureInput.value = "";
  }

  $effect(() => () => {
    clearInterval(timer);
    stream?.getTracks().forEach((t) => t.stop());
  });
</script>

<div class="rec">
  {#if canRecord}
    {#if recording}
      <button class="primary stop" onclick={stop}>Selesai</button>
      <span class="mono time">{clock(elapsed)}</span>
      <div class="bar grow" title="Level masukan"><i class:hot={level > 0.95} style="width:{Math.min(100, level * 100)}%"></i></div>
    {:else}
      <button class="primary" onclick={start} {disabled}>Mulai rekam</button>
      <span class="muted small grow">Dekatkan ke speaker atau instrumen. Hasil paling akurat dari rekaman 20 detik atau lebih, tanpa banyak suara obrolan.</span>
    {/if}
  {:else}
    <button class="primary" onclick={() => captureInput?.click()} {disabled}>Rekam dengan perekam HP</button>
    <span class="muted small grow">Perekam bawaan browser butuh HTTPS. Aktifkan mode LAN aman di Pengaturan, atau pakai perekam HP lewat tombol ini.</span>
    <input bind:this={captureInput} type="file" accept="audio/*" capture hidden onchange={(e) => fromCapture(e.currentTarget.files)} />
  {/if}
</div>

<style>
  .rec {
    display: flex;
    align-items: center;
    gap: 0.8rem;
    flex-wrap: wrap;
    min-height: 2.6rem;
  }
  .time {
    font-size: 1.1rem;
  }
  .stop {
    background: var(--bad);
    border-color: var(--bad);
    color: #fff;
  }
  .bar i.hot {
    background: var(--bad);
  }
</style>
