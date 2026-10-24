// Typed client for the ChordKu server, plus the single SSE subscription.

import type { DrumEvents } from "./chart/drums";
import type { Analysis, Edit } from "./chart/model";
import type { Key } from "./theory";

export interface Song {
  id: string;
  title: string;
  artist: string;
  duration_ms: number;
  src_kind: "url" | "file" | "mic";
  src_url: string | null;
  thumbnail: string | null;
  fav: boolean;
  created_at: number;
  key: (Key & { confidence: number }) | null;
  bpm: number | null;
  has_chart: boolean;
  has_stems: boolean;
  tags: string[];
}

export interface Job {
  id: string;
  song_id: string;
  kind: "analyze" | "separate";
  state: "queued" | "running" | "succeeded" | "failed" | "canceled";
  stage: string;
  progress: number;
  error: string | null;
  updated_at: number;
}

export interface StemFile {
  name: string;
  file: string;
  bytes: number;
  channels: number;
}

export interface SongDetail {
  song: Song;
  analysis: Analysis | null;
  edits: Edit[];
  prefs: Record<string, unknown>;
  stems: StemFile[];
  /** Kick / snare / hi-hat hits from the drum stem, once stems exist. */
  drums: DrumEvents | null;
  jobs: Job[];
}

export interface ModelStatus {
  id: string;
  title: string;
  license: string;
  source: string;
  required: boolean;
  installed: boolean;
  missing: string[];
  downloadable: boolean;
  download_bytes: number;
}

export interface SystemInfo {
  version: string;
  onnxruntime: string;
  data_dir: string;
  media_bytes: number;
  free_memory: number;
  max_duration_secs: number;
  max_upload_bytes: number;
  models: ModelStatus[];
  accel: {
    calibrating: boolean;
    calibration: {
      plan: { chords: string; beats: string; stems: string };
      stems_measured: boolean;
      trials: { model: string; device: string; available: boolean; run_s: number; fidelity: number; accepted: boolean; note: string }[];
    } | null;
  };
  cuda: { supported: boolean; nvidia_gpu: boolean; installed: boolean; dir: string; download_bytes: number };
  ytdlp: { installed: boolean; version: string | null; path: string | null; js_runtime: string | null };
}

export interface Setlist {
  id: number;
  name: string;
  count: number;
}

export interface LanInfo {
  enabled: boolean;
  running: boolean;
  port: number;
  addresses: string[];
  pair_url: string | null;
  pair_qr_svg: string | null;
  pair_expires_secs: number | null;
  error: string | null;
}

export interface Device {
  id: string;
  name: string;
  created_at: number;
  last_seen: number;
}

export interface LanState {
  lan: LanInfo;
  devices: Device[];
}

export type ServerEvent =
  | { type: "job"; job: Job }
  | { type: "song"; id: string }
  | { type: "song_deleted"; id: string }
  | { type: "model"; id: string; progress: number; error: string | null; done: boolean }
  | { type: "system" };

export class ApiError extends Error {
  constructor(message: string, readonly status: number) {
    super(message);
  }
}

async function request<T>(method: string, path: string, body?: unknown): Promise<T> {
  let res: Response;
  try {
    res = await fetch(`/api${path}`, {
      method,
      headers: body === undefined ? undefined : { "Content-Type": "application/json" },
      body: body === undefined ? undefined : JSON.stringify(body),
    });
  } catch {
    throw new ApiError("Server ChordKu tidak merespons. Pastikan aplikasinya masih berjalan.", 0);
  }
  const text = await res.text();
  let data: unknown = null;
  try {
    data = text ? JSON.parse(text) : null;
  } catch {
    /* non-JSON error body */
  }
  if (!res.ok) {
    const message = (data as { error?: string } | null)?.error ?? text ?? `HTTP ${res.status}`;
    throw new ApiError(message, res.status);
  }
  return data as T;
}

export const api = {
  songs: (q = "", tag = "", fav = false) =>
    request<{ songs: Song[]; tags: string[] }>("GET", `/songs?q=${encodeURIComponent(q)}&tag=${encodeURIComponent(tag)}${fav ? "&fav=1" : ""}`),
  song: (id: string) => request<SongDetail>("GET", `/songs/${id}`),
  /** `separate`: queue a separation once the song is in. `warning`: why that could not happen. */
  addUrl: (url: string, separate = false) =>
    request<{ existing: boolean; retried?: boolean; song: Song; job?: Job; warning?: string | null }>("POST", "/songs/url", { url, separate }),
  patchSong: (id: string, patch: Partial<Pick<Song, "title" | "artist" | "fav" | "tags">>) =>
    request<{ song: Song }>("PATCH", `/songs/${id}`, patch),
  deleteSong: (id: string) => request<{ ok: boolean }>("DELETE", `/songs/${id}`),
  addEdit: (id: string, edit: Omit<Edit, "id">) => request<{ id: number }>("POST", `/songs/${id}/edits`, edit),
  deleteEdit: (id: string, editId: number) => request<{ ok: boolean }>("DELETE", `/songs/${id}/edits/${editId}`),
  clearEdits: (id: string) => request<{ ok: boolean }>("DELETE", `/songs/${id}/edits`),
  setPrefs: (id: string, prefs: unknown) => request<{ ok: boolean }>("PUT", `/songs/${id}/prefs`, prefs),
  reanalyze: (id: string) => request<{ job: Job }>("POST", `/songs/${id}/analyze`),
  separate: (id: string) => request<{ job: Job }>("POST", `/songs/${id}/separate`),
  deleteStems: (id: string) => request<{ ok: boolean }>("DELETE", `/songs/${id}/stems`),
  cancelJob: (id: string) => request<{ canceled: boolean }>("POST", `/jobs/${id}/cancel`),
  system: () => request<SystemInfo>("GET", "/system"),
  downloadModel: (id: string) => request<{ started: boolean }>("POST", `/system/models/${id}/download`),
  installCuda: () => request<{ started: boolean }>("POST", "/system/cuda/install"),
  calibrate: () => request<{ started: boolean }>("POST", "/system/accel/calibrate"),
  updateYtdlp: () => request<{ version: string }>("POST", "/system/ytdlp/update"),
  setlists: () => request<{ setlists: Setlist[] }>("GET", "/setlists"),
  createSetlist: (name: string) => request<{ id: number; setlists: Setlist[] }>("POST", "/setlists", { name }),
  setlist: (id: number) => request<{ setlist: Setlist; songs: Song[] }>("GET", `/setlists/${id}`),
  renameSetlist: (id: number, name: string) => request<{ setlists: Setlist[] }>("PATCH", `/setlists/${id}`, { name }),
  deleteSetlist: (id: number) => request<{ setlists: Setlist[] }>("DELETE", `/setlists/${id}`),
  orderSetlist: (id: number, song_ids: string[]) => request<{ songs: Song[] }>("PUT", `/setlists/${id}/songs`, { song_ids }),
  /** The setlist as one self-contained web page (a download, not a JSON call). */
  shareSetlistUrl: (id: number, audio: "" | "mix" | "light" | "stems" = "") => `/api/setlists/${id}/share.html${audio ? `?audio=${audio}` : ""}`,
  addToSetlist: (id: number, song_id: string) => request<{ setlists: Setlist[] }>("POST", `/setlists/${id}/songs`, { song_id }),
  session: () => request<{ lan: boolean; paired: boolean }>("GET", "/session"),
  pair: (code: string, name: string) => request<{ ok: boolean; name: string }>("POST", "/pair", { code, name }),
  lan: () => request<LanState>("GET", "/system/lan"),
  setLan: (enabled: boolean) => request<LanState>("POST", "/system/lan", { enabled }),
  newPairCode: () => request<LanState>("POST", "/system/lan/pair-code"),
  deleteDevice: (id: string) => request<LanState>("DELETE", `/system/lan/devices/${id}`),
  mediaUrl: (songId: string, file: string) => `/api/media/${songId}/${file}`,

  /** Multipart upload with progress (fetch cannot report upload progress). */
  upload(file: Blob, fields: { title?: string; kind?: "file" | "mic"; filename?: string; separate?: boolean }, onProgress?: (f: number) => void) {
    return new Promise<{ existing: boolean; song: Song; job?: Job; warning?: string | null }>((resolve, reject) => {
      const form = new FormData();
      if (fields.title) form.append("title", fields.title);
      if (fields.separate) form.append("separate", "1");
      form.append("kind", fields.kind ?? "file");
      form.append("file", file, fields.filename ?? (file instanceof File ? file.name : "rekaman.webm"));
      const xhr = new XMLHttpRequest();
      xhr.open("POST", "/api/songs/upload");
      xhr.upload.onprogress = (e) => e.lengthComputable && onProgress?.(e.loaded / e.total);
      xhr.onerror = () => reject(new ApiError("Unggahan gagal: server tidak merespons.", 0));
      xhr.onload = () => {
        let data: any = null;
        try {
          data = JSON.parse(xhr.responseText);
        } catch {
          /* ignore */
        }
        if (xhr.status >= 200 && xhr.status < 300) resolve(data);
        else reject(new ApiError(data?.error ?? `HTTP ${xhr.status}`, xhr.status));
      };
      xhr.send(form);
    });
  },
};

type Listener = (e: ServerEvent) => void;
const listeners = new Set<Listener>();
let source: EventSource | null = null;

/** Subscribe to server events. The connection is shared and reconnects by itself. */
export function onServerEvent(fn: Listener): () => void {
  listeners.add(fn);
  if (!source) {
    source = new EventSource("/api/events");
    source.onmessage = (m) => {
      try {
        const event = JSON.parse(m.data) as ServerEvent;
        listeners.forEach((l) => l(event));
      } catch {
        /* ignore malformed event */
      }
    };
    // After a reconnect we may have missed events: ask everyone to refresh.
    let hadError = false;
    source.onerror = () => (hadError = true);
    source.onopen = () => {
      if (hadError) listeners.forEach((l) => l({ type: "system" }));
      hadError = false;
    };
  }
  return () => {
    listeners.delete(fn);
    if (!listeners.size && source) {
      source.close();
      source = null;
    }
  };
}
