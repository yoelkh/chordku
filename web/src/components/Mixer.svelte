<script lang="ts">
  import { STEM_LABELS } from "../lib/i18n/id";

  export interface Channel {
    name: string;
    gain: number;
    muted: boolean;
    solo: boolean;
  }

  let { channels, onchange }: { channels: Channel[]; onchange: (channels: Channel[]) => void } = $props();

  function update(i: number, patch: Partial<Channel>) {
    onchange(channels.map((c, k) => (k === i ? { ...c, ...patch } : c)));
  }
</script>

<div class="mixer">
  {#each channels as c, i (c.name)}
    <div class="ch" class:dim={c.muted}>
      <span class="name ellipsis">{STEM_LABELS[c.name] ?? c.name}</span>
      <input type="range" min="0" max="1.5" step="0.01" value={c.gain} oninput={(e) => update(i, { gain: +e.currentTarget.value })} aria-label="Volume {STEM_LABELS[c.name] ?? c.name}" />
      <button class="icon small" class:on={c.muted} onclick={() => update(i, { muted: !c.muted })} aria-pressed={c.muted} title="Bisukan">M</button>
      <button class="icon small" class:on={c.solo} onclick={() => update(i, { solo: !c.solo })} aria-pressed={c.solo} title="Solo">S</button>
    </div>
  {/each}
</div>

<style>
  .mixer {
    display: grid;
    gap: 0.4rem;
    grid-template-columns: repeat(auto-fit, minmax(min(100%, 17rem), 1fr));
  }
  .ch {
    display: grid;
    grid-template-columns: 8.5rem 1fr auto auto;
    align-items: center;
    gap: 0.45rem;
  }
  .ch.dim .name {
    color: var(--muted);
    text-decoration: line-through;
  }
  .name {
    font-weight: 600;
    font-size: 0.92rem;
  }
  input {
    width: 100%;
    min-width: 0;
  }
</style>
