// Teks bersama (Bahasa Indonesia). Teks yang hanya dipakai satu komponen ada di komponennya.

import type { Job } from "../api";

const STAGES: Record<string, string> = {
  download: "Mengunduh audio",
  decode: "Membaca audio",
  analyze: "Mengenali chord dan ketukan",
  encode: "Menyiapkan audio pemutar",
  model_check: "Memeriksa model",
  segments: "Memisahkan vokal dan instrumen",
};

export const DEVICE_LABELS: Record<string, string> = {
  cpu: "CPU",
  cuda: "GPU NVIDIA (CUDA)",
  gpu_high: "GPU NVIDIA (DirectML)",
  gpu_low: "GPU Intel Arc (DirectML)",
  npu: "NPU Intel AI Boost",
};

export function jobLabel(job: Job): string {
  if (job.state === "queued") return job.kind === "separate" ? "Menunggu giliran pisah stem" : "Menunggu giliran";
  if (job.state === "failed") return job.error ?? "Gagal";
  if (job.state === "canceled") return "Dibatalkan";
  if (job.state === "succeeded") return "Selesai";
  const [stage, device] = job.stage.split(":");
  const where = device ? ` di ${DEVICE_LABELS[device] ?? device}` : "";
  return (STAGES[stage] ?? (job.kind === "separate" ? "Memisahkan stem" : "Menganalisis")) + where;
}

export function clock(ms: number): string {
  const s = Math.max(0, Math.floor(ms / 1000));
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, "0")}`;
}

export function bytes(n: number): string {
  if (n >= 1e9) return `${(n / 1e9).toFixed(1)} GB`;
  if (n >= 1e6) return `${(n / 1e6).toFixed(0)} MB`;
  return `${Math.max(1, Math.round(n / 1e3))} KB`;
}

export const STEM_LABELS: Record<string, string> = {
  mix: "Lagu asli",
  vocals: "Vokal",
  drums: "Drum",
  bass: "Bass",
  other: "Gitar, piano, lainnya",
  vocals_mono: "Vokal",
  backing_mono: "Iringan",
};
