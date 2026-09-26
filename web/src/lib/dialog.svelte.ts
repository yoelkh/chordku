// In-app replacements for window.prompt / window.confirm. The browser dialogs are blocked in
// some contexts (embedded browsers, "prevent this page from creating additional dialogs",
// some PWA shells), and then a setlist or a rename silently does nothing.

export interface DialogRequest {
  kind: "prompt" | "confirm";
  title: string;
  /** Extra line under the title. */
  text?: string;
  value: string;
  placeholder?: string;
  ok: string;
  danger?: boolean;
  /** Accept an empty answer (default: the OK button stays disabled until something is typed). */
  allowEmpty?: boolean;
  resolve: (result: string | null) => void;
}

export const dialog = $state<{ current: DialogRequest | null }>({ current: null });

/** Ask for a line of text; resolves null when cancelled. */
export function askText(title: string, value = "", opts: { placeholder?: string; ok?: string; text?: string; allowEmpty?: boolean } = {}): Promise<string | null> {
  return new Promise((resolve) => {
    dialog.current?.resolve(null);
    dialog.current = { kind: "prompt", title, value, placeholder: opts.placeholder, ok: opts.ok ?? "OK", text: opts.text, allowEmpty: opts.allowEmpty, resolve };
  });
}

/** Yes / no; resolves true on confirmation. */
export function askConfirm(title: string, opts: { text?: string; ok?: string; danger?: boolean } = {}): Promise<boolean> {
  return new Promise((resolve) => {
    dialog.current?.resolve(null);
    dialog.current = { kind: "confirm", title, value: "", ok: opts.ok ?? "Ya", text: opts.text, danger: opts.danger, resolve: (r) => resolve(r !== null) };
  });
}

export function settle(result: string | null) {
  const c = dialog.current;
  dialog.current = null;
  c?.resolve(result);
}
