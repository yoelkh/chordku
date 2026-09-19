// Tiny global toast store (Svelte 5 runes in a module).

export interface Toast {
  id: number;
  text: string;
  kind: "info" | "error" | "ok";
}

let next = 1;
export const toasts = $state<Toast[]>([]);

export function toast(text: string, kind: Toast["kind"] = "info", ms = 4500) {
  const id = next++;
  toasts.push({ id, text, kind });
  setTimeout(() => dismiss(id), kind === "error" ? ms * 2 : ms);
}

export function dismiss(id: number) {
  const i = toasts.findIndex((t) => t.id === id);
  if (i >= 0) toasts.splice(i, 1);
}

export function fail(e: unknown) {
  toast(e instanceof Error ? e.message : String(e), "error");
}
