<script lang="ts">
  // Modal for the askText / askConfirm store. Mounted once in App.svelte.
  import { dialog, settle } from "../lib/dialog.svelte";

  let el = $state<HTMLDialogElement>();
  let input = $state<HTMLInputElement>();
  let value = $state("");

  $effect(() => {
    const c = dialog.current;
    if (!el) return;
    if (c) {
      value = c.value;
      if (!el.open) el.showModal();
      queueMicrotask(() => (c.kind === "prompt" ? (input?.focus(), input?.select()) : el?.querySelector<HTMLButtonElement>("button.primary")?.focus()));
    } else if (el.open) {
      el.close();
    }
  });

  function submit(e: Event) {
    e.preventDefault();
    const c = dialog.current;
    if (!c) return;
    if (c.kind === "prompt" && !value.trim() && !c.allowEmpty) return;
    settle(c.kind === "prompt" ? value.trim() : "");
  }
</script>

<dialog bind:this={el} class="card" oncancel={(e) => (e.preventDefault(), settle(null))} onclick={(e) => e.target === el && settle(null)}>
  {#if dialog.current}
    {@const c = dialog.current}
    <form onsubmit={submit}>
      <h3>{c.title}</h3>
      {#if c.text}<p class="muted small">{c.text}</p>{/if}
      {#if c.kind === "prompt"}
        <input bind:this={input} type="text" bind:value placeholder={c.placeholder ?? ""} maxlength="120" autocomplete="off" />
      {/if}
      <div class="row end">
        <button type="button" class="ghost" onclick={() => settle(null)}>Batal</button>
        <button type="submit" class="primary" class:danger={c.danger} disabled={c.kind === "prompt" && !value.trim() && !c.allowEmpty}>{c.ok}</button>
      </div>
    </form>
  {/if}
</dialog>

<style>
  dialog {
    border: 1px solid var(--line);
    padding: 1rem 1.1rem;
    width: min(92vw, 26rem);
    color: inherit;
  }
  dialog::backdrop {
    background: rgb(0 0 0 / 0.55);
  }
  form {
    display: grid;
    gap: 0.7rem;
  }
  h3 {
    margin: 0;
    font-size: 1.05rem;
  }
  p {
    margin: 0;
  }
  input {
    width: 100%;
  }
  .end {
    justify-content: flex-end;
    gap: 0.5rem;
  }
  button.primary.danger {
    background: var(--bad);
    border-color: var(--bad);
    color: #fff;
  }
</style>
