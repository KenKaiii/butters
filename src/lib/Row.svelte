<script lang="ts">
  import type { Outcome, Suggestion } from "./api";
  import { bytes } from "./format";
  import Check from "./Check.svelte";
  import Icon from "./Icon.svelte";
  import { dissolve, pour } from "./motion";

  let {
    s,
    selected,
    pending,
    melting = false,
    result,
    onToggle,
    onIgnore,
    onForce,
  }: {
    s: Suggestion;
    selected: boolean;
    pending: boolean;
    melting?: boolean;
    result: Outcome | undefined;
    onToggle: (on: boolean) => void;
    onIgnore: () => void;
    onForce: () => void;
  } = $props();

  let confirmForce = $state(false);
  const canForce = $derived(
    result &&
      (result.status === "still_running" || result.status === "needs_force") &&
      s.kind !== "docker_container" &&
      s.kind !== "ollama_model",
  );
  const inputId = $derived(`pick-${s.key}`);
  // One extra fact after the reason: a caution if there is one, else where it lives.
  const aside = $derived(s.note ?? (!s.polite ? "Force close only" : s.detail));
</script>

<div class="pat" class:melting>
<div class="row" class:picked={selected} class:pending aria-busy={pending}>
  <Check id={inputId} checked={selected} disabled={pending} onchange={onToggle} />
  <label class="text" for={inputId} title={[s.title, s.reason, aside].filter(Boolean).join(" · ")}>
    <span class="title">{s.title}</span>
    <span class="meta">{s.reason}{#if aside}{" · "}<span class:flag={!!s.note || !s.polite}>{aside}</span>{/if}</span>
  </label>
  <span class="mem">{bytes(s.memory)}</span>
  <button class="ghost" type="button" aria-label={`Ignore ${s.title}`} title="Ignore" disabled={pending} onclick={onIgnore}>
    <Icon name="eyeOff" />
  </button>

  {#if pending}
    <p class="result softening" role="status" in:dissolve>Closing…</p>
  {:else if result && result.status !== "closed"}
    <div class="result" role="status" in:pour={{ y: -4 }}>
      {#if canForce && confirmForce}
        <span class="msg warn">Unsaved work is lost</span>
        <button type="button" class="btn" onclick={() => (confirmForce = false)}>Cancel</button>
        <button type="button" class="btn danger" onclick={() => { confirmForce = false; onForce(); }}>Force close</button>
      {:else}
        <span class="msg" title={result.message}>{result.message}</span>
        {#if canForce}<button type="button" class="btn" onclick={() => (confirmForce = true)}>Force close…</button>{/if}
      {/if}
    </div>
  {/if}
</div>
  <!-- Butter that runs off the bottom edge as the row melts. -->
  <span class="drips" aria-hidden="true"><i></i><i></i><i></i><i></i></span>
</div>

<style>
  .pat {
    position: relative;
  }
  .row {
    display: grid;
    grid-template-columns: 24px minmax(0, 1fr) auto 28px;
    column-gap: 10px;
    align-items: center;
    padding: 8px var(--inset);
    border-radius: var(--r-pat);
    background: transparent;
    color: var(--ink);
    transition:
      background-color var(--t-soft) var(--butter-ease),
      opacity var(--t-soft) var(--butter-ease),
      filter var(--t-soft) var(--butter-ease);
  }
  .row:hover {
    background: var(--surface-soft);
    color: var(--ink);
  }
  .row.picked {
    background: var(--surface-picked);
    color: var(--ink);
  }
  /* While closing, the row starts to soften before it melts away. */
  .row.pending {
    opacity: 0.7;
    filter: blur(0.4px);
  }
  .text {
    display: flex;
    flex-direction: column;
    min-width: 0;
    cursor: pointer;
  }
  .title,
  .meta,
  .msg {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .title {
    font-weight: 600;
    font-size: 13px;
    letter-spacing: -0.015em;
  }
  .meta {
    color: var(--muted);
    font-size: 12px;
  }
  .flag {
    color: var(--ink);
    font-weight: 500;
  }
  .mem {
    font-weight: 600;
    font-size: 13px;
    white-space: nowrap;
  }
  .ghost {
    width: 28px;
    height: 28px;
    display: grid;
    place-items: center;
    border: 0;
    border-radius: 8px;
    background: transparent;
    color: var(--muted);
    cursor: pointer;
    transition:
      color var(--t-soft) var(--butter-ease),
      background-color var(--t-soft) var(--butter-ease),
      transform var(--t-quick) var(--butter-ease);
  }
  .ghost:hover:not(:disabled) {
    color: var(--ink);
    background: var(--hairline);
  }
  .ghost:active:not(:disabled) {
    transform: scale(0.9);
  }
  .ghost:disabled {
    opacity: 0.4;
    cursor: default;
  }
  .result {
    grid-column: 2 / -1;
    display: flex;
    align-items: center;
    gap: 8px;
    margin: 6px 0 0;
    font-size: 12px;
  }
  .msg {
    flex: 1;
    min-width: 0;
  }
  .softening {
    color: var(--muted);
  }
  .warn {
    color: var(--danger);
  }
  .btn {
    height: 28px;
    padding: 0 10px;
    flex: none;
    border: 1px solid var(--hairline);
    border-radius: var(--r-control);
    background: var(--surface);
    color: var(--ink);
    font-weight: 500;
    cursor: pointer;
    transition:
      background-color var(--t-soft) var(--butter-ease),
      transform var(--t-quick) var(--butter-ease);
  }
  .btn:hover {
    background: var(--surface-soft);
    color: var(--ink);
  }
  .btn:active {
    transform: scale(0.97);
  }
  .btn.danger {
    background: var(--danger-fill);
    border-color: var(--danger-fill);
    color: var(--danger-ink);
    font-weight: 600;
  }
  .btn:disabled {
    opacity: 0.5;
    cursor: default;
  }
  /* Melting: the row warms to butter, holds a beat, then slumps into a puddle
     while drips run off its bottom edge. Content softens away first. */
  .melting .row {
    pointer-events: none;
    transform-origin: 50% 100%;
    animation: row-melt 1000ms cubic-bezier(0.5, 0, 0.75, 0.25) forwards;
  }
  .melting .row > :global(*) {
    animation: content-melt 560ms var(--butter-ease) forwards;
  }
  @keyframes row-melt {
    0% {
      background: var(--surface-picked);
      transform: none;
      opacity: 1;
    }
    22% {
      background: var(--butter-hi);
      transform: scale(1.01, 1.02);
    }
    70% {
      background: var(--butter);
      border-radius: 20px;
      opacity: 1;
    }
    100% {
      background: var(--butter);
      border-radius: 40px;
      transform: translateY(4px) scale(1.04, 0.08);
      opacity: 0;
    }
  }
  @keyframes content-melt {
    to {
      opacity: 0;
      filter: blur(5px);
      transform: translateY(8px) scaleY(0.7);
    }
  }
  .drips {
    position: absolute;
    left: 0;
    right: 0;
    bottom: 0;
    height: 0;
    pointer-events: none;
  }
  .drips i {
    position: absolute;
    top: -2px;
    width: 7px;
    height: 24px;
    border-radius: 0 0 4px 4px;
    background: linear-gradient(var(--butter), var(--butter-deep));
    transform-origin: 50% 0;
    transform: scaleY(0);
    opacity: 0;
  }
  .drips i::after {
    /* the bead at the tip */
    content: "";
    position: absolute;
    left: -1.5px;
    bottom: -4px;
    width: 10px;
    height: 10px;
    border-radius: 50%;
    background: var(--butter-deep);
  }
  .drips i:nth-child(1) {
    left: 18%;
  }
  .drips i:nth-child(2) {
    left: 41%;
    width: 5px;
  }
  .drips i:nth-child(3) {
    left: 63%;
  }
  .drips i:nth-child(4) {
    left: 84%;
    width: 5px;
  }
  .melting .drips i {
    animation: drip 1000ms cubic-bezier(0.55, 0, 0.6, 1) forwards;
  }
  .melting .drips i:nth-child(2) {
    animation-delay: 80ms;
  }
  .melting .drips i:nth-child(3) {
    animation-delay: 30ms;
  }
  .melting .drips i:nth-child(4) {
    animation-delay: 130ms;
  }
  @keyframes drip {
    0%,
    35% {
      transform: scaleY(0);
      opacity: 0;
    }
    45% {
      opacity: 1;
    }
    80% {
      transform: translateY(0) scaleY(1);
      opacity: 1;
    }
    100% {
      transform: translateY(14px) scaleY(1.15);
      opacity: 0;
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .melting .row {
      opacity: 0;
    }
    .drips {
      display: none;
    }
    .ghost:active:not(:disabled),
    .btn:active {
      transform: none;
    }
  }
</style>
