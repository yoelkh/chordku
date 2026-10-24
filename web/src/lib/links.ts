// Pasted text -> song links: what the bulk-add box accepts.

export interface ParsedLinks {
  /** Normalised, de-duplicated, in paste order. */
  urls: string[];
  /** Links that appeared more than once (counted once). */
  duplicates: number;
}

const URL_RE = /\b(?:https?:\/\/|www\.|m\.youtube\.com\/|music\.youtube\.com\/|youtu\.be\/|youtube\.com\/)[^\s<>"'`]+/gi;

/** The YouTube video id of a link, or null when it is not a YouTube video link. */
export function youtubeId(url: string): string | null {
  try {
    const u = new URL(url);
    const host = u.hostname.replace(/^(www|m|music)\./, "");
    let id: string | null = null;
    if (host === "youtu.be") id = u.pathname.slice(1).split("/")[0];
    else if (host === "youtube.com" || host === "youtube-nocookie.com")
      id = u.searchParams.get("v") ?? u.pathname.match(/^\/(?:shorts|embed|live|v)\/([^/?]+)/)?.[1] ?? null;
    return id && /^[\w-]{11}$/.test(id) ? id : null;
  } catch {
    return null;
  }
}

/** One canonical form per video, so the same song pasted twice (or with a playlist,
 *  timestamp or share tracking parameter) is fetched once. Other sites pass through. */
export function normalizeLink(raw: string): string | null {
  let s = raw.trim().replace(/[),.;:!?\]}>]+$/, "");
  if (!/^https?:\/\//i.test(s)) s = `https://${s}`;
  let url: URL;
  try {
    url = new URL(s);
  } catch {
    return null;
  }
  if (url.protocol !== "http:" && url.protocol !== "https:") return null;
  const id = youtubeId(url.href);
  if (id) return `https://www.youtube.com/watch?v=${id}`;
  url.hash = "";
  return url.href;
}

export function parseLinks(text: string): ParsedLinks {
  const seen = new Set<string>();
  const urls: string[] = [];
  let duplicates = 0;
  for (const m of text.match(URL_RE) ?? []) {
    const n = normalizeLink(m);
    if (!n) continue;
    if (seen.has(n)) {
      duplicates++;
      continue;
    }
    seen.add(n);
    urls.push(n);
  }
  return { urls, duplicates };
}

/** "Sesi 2 24/10/2026" when "Sesi 1 24/10/2026" already exists. */
export function nextSessionName(existing: string[], date = new Date()): string {
  const d = `${String(date.getDate()).padStart(2, "0")}/${String(date.getMonth() + 1).padStart(2, "0")}/${date.getFullYear()}`;
  const taken = new Set(
    existing.flatMap((name) => {
      const m = name.trim().match(/^Sesi\s+(\d+)\s+(\S+)$/i);
      return m && m[2] === d ? [Number(m[1])] : [];
    }),
  );
  let n = 1;
  while (taken.has(n)) n++;
  return `Sesi ${n} ${d}`;
}
