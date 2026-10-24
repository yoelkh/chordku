// "Pisahkan vokal & instrumen otomatis": one switch for every way a song comes in (link,
// pasted list, file, recording), remembered on this device.

const KEY = "chordku.autoSeparate";

function read(): boolean {
  try {
    return (localStorage.getItem(KEY) ?? "1") === "1";
  } catch {
    return true;
  }
}

export const autoSeparate = $state({ on: read() });

export function setAutoSeparate(on: boolean) {
  autoSeparate.on = on;
  try {
    localStorage.setItem(KEY, on ? "1" : "0");
  } catch {
    /* private mode: for this session only */
  }
}
