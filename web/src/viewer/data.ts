// What the server embeds into viewer.html, and how the page reads it.
import type { DrumEvents } from "../lib/chart/drums";
import type { Analysis, Edit } from "../lib/chart/model";

export interface ViewerSong {
  id: string;
  title: string;
  artist: string;
  duration_ms: number;
  src_url: string | null;
  thumbnail: string | null;
  analysis: Analysis;
  edits: Edit[];
  drums?: DrumEvents | null;
  /** Playback file(s) as data URIs when the page was exported with audio. `mix` is the
   *  single-file form of earlier pages. */
  audio?: {
    kind?: "mix" | "light" | "stems";
    tracks?: { name: string; url: string }[];
    /** Stems were requested but this song had not been separated. */
    fallback?: boolean;
    mix?: string;
  } | null;
  /** The owner's view prefs at export time: the starting point for every reader. */
  prefs: Record<string, unknown>;
}

export interface ViewerData {
  app: string;
  version: string;
  setlist: { id: number; name: string };
  exported_at: number;
  songs: ViewerSong[];
}

/** Embedded JSON first; during development `?setlist=<id>` fetches it from the dev server. */
export async function loadData(): Promise<ViewerData | null> {
  const el = document.getElementById("chordku-setlist");
  if (el?.textContent?.trim()) return JSON.parse(el.textContent) as ViewerData;
  const id = new URLSearchParams(location.search).get("setlist");
  if (!id) return null;
  const r = await fetch(`/api/setlists/${encodeURIComponent(id)}/share.json`);
  if (!r.ok) throw new Error(`setlist ${id}: HTTP ${r.status}`);
  return (await r.json()) as ViewerData;
}

/** YouTube video id from the forms yt-dlp accepts (watch, youtu.be, shorts, embed, music). */
export function youtubeId(url: string | null): string | null {
  if (!url) return null;
  try {
    const u = new URL(url);
    const host = u.hostname.replace(/^www\.|^m\.|^music\./, "");
    const id = host === "youtu.be" ? u.pathname.slice(1) : host === "youtube.com" ? (u.searchParams.get("v") ?? u.pathname.match(/^\/(?:shorts|embed|live)\/([^/?]+)/)?.[1] ?? null) : null;
    return id && /^[\w-]{11}$/.test(id) ? id : null;
  } catch {
    return null;
  }
}
