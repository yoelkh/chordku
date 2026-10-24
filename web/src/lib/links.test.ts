import { describe, expect, it } from "vitest";
import { nextSessionName, normalizeLink, parseLinks } from "./links";

describe("pasted links", () => {
  it("finds links in messy text, normalises YouTube forms and drops duplicates", () => {
    const text = `Lagu pembuka: https://www.youtube.com/watch?v=GKvYwq4J6TM&list=RDGKvYwq4J6TM&index=2
      2) https://youtu.be/f2oxGYpuLkw?si=abc123
      3. youtube.com/watch?v=GKvYwq4J6TM (sama seperti nomor 1)
      https://m.youtube.com/watch?v=dQw4w9WgXcQ&t=42s,
      https://music.youtube.com/watch?v=aBcDeFgHiJk&feature=share
      https://www.youtube.com/shorts/ZYXWVUTSRQP
      soundcloud: https://soundcloud.com/artist/track#t=1:00
      bukan link: www, http:// , ftp://x.y`;
    const { urls, duplicates } = parseLinks(text);
    expect(urls).toEqual([
      "https://www.youtube.com/watch?v=GKvYwq4J6TM",
      "https://www.youtube.com/watch?v=f2oxGYpuLkw",
      "https://www.youtube.com/watch?v=dQw4w9WgXcQ",
      "https://www.youtube.com/watch?v=aBcDeFgHiJk",
      "https://www.youtube.com/watch?v=ZYXWVUTSRQP",
      "https://soundcloud.com/artist/track",
    ]);
    expect(duplicates).toBe(1);
  });

  it("rejects what is not a web link", () => {
    expect(normalizeLink("javascript:alert(1)")).toBeNull();
    expect(parseLinks("tidak ada tautan di sini").urls).toEqual([]);
  });

  it("numbers sessions per day", () => {
    const day = new Date(2026, 9, 24);
    expect(nextSessionName([], day)).toBe("Sesi 1 24/10/2026");
    expect(nextSessionName(["Sesi 1 24/10/2026", "Sesi 2 24/10/2026", "Sesi 1 27/09/2026"], day)).toBe("Sesi 3 24/10/2026");
    expect(nextSessionName(["Sesi 2 24/10/2026"], day)).toBe("Sesi 1 24/10/2026");
  });
});
