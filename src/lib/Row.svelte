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
    onSlice,
  }: {
    s: Suggestion;
    selected: boolean;
    pending: boolean;
    melting?: boolean;
    result: Outcome | undefined;
    onToggle: (on: boolean) => void;
    onIgnore: () => void;
    onForce: () => void;
    /** Knife gesture: called after the user swipes and confirms "Slice". */
    onSlice?: () => void;
  } = $props();

  // Butter knife: drag the row sideways to slice it off. Past the threshold it
  // stays open with a "Slice?" button (a confirm, so a stray trackpad swipe
  // can never close anything); short of it, it springs back. Keyboard users
  // have the checkbox + Close, so the gesture is never the only way.
  const ARM = 0.32;
  let rowEl: HTMLDivElement | undefined = $state();
  let dx = $state(0);
  let dragging = $state(false);
  let armed = $state(false);
  let start: { x: number; y: number; id: number } | null = null;
  let suppressClick = false;

  function width() {
    return rowEl?.getBoundingClientRect().width ?? 340;
  }
  function down(e: PointerEvent) {
    if (pending || melting || !onSlice || e.button !== 0) return;
    if ((e.target as Element).closest("button, input")) return;
    start = { x: e.clientX, y: e.clientY, id: e.pointerId };
  }
  function move(e: PointerEvent) {
    if (!start || e.pointerId !== start.id) return;
    const mx = e.clientX - start.x;
    const my = e.clientY - start.y;
    if (!dragging) {
      if (Math.abs(mx) < 8 || Math.abs(mx) < Math.abs(my) * 1.5 || mx < 0) return;
      dragging = true;
      armed = false;
      rowEl?.setPointerCapture(e.pointerId);
    }
    // A little resistance past the arm point, like dragging through cold butter.
    const w = width();
    const raw = Math.max(0, mx);
    dx = raw < w * ARM ? raw : w * ARM + (raw - w * ARM) * 0.35;
  }
  function up(e: PointerEvent) {
    if (!start || e.pointerId !== start.id) return;
    start = null;
    if (!dragging) return;
    dragging = false;
    suppressClick = true;
    if (dx >= width() * ARM * 0.92) {
      armed = true;
      dx = width() * ARM;
    } else {
      armed = false;
      dx = 0;
    }
  }
  function swallow(e: MouseEvent) {
    // The click after a drag must not tick the checkbox.
    if (suppressClick) {
      suppressClick = false;
      e.preventDefault();
      e.stopPropagation();
    }
  }
  export function disarm() {
    armed = false;
    dx = 0;
  }
  function slice() {
    armed = false;
    dx = 0;
    onSlice?.();
  }
  $effect(() => {
    if (!armed) return;
    const off = (e: Event) => {
      if (!rowEl?.parentElement?.contains(e.target as Node)) disarm();
    };
    const esc = (e: KeyboardEvent) => e.key === "Escape" && (e.stopPropagation(), disarm());
    document.addEventListener("pointerdown", off, true);
    window.addEventListener("keydown", esc, true);
    return () => {
      document.removeEventListener("pointerdown", off, true);
      window.removeEventListener("keydown", esc, true);
    };
  });

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

<div class="pat" class:melting class:sliding={dragging || armed}>
<!-- Revealed behind the row as it slides: the knife's cut. -->
<div class="knife" aria-hidden={!armed} class:armed style:--reveal={Math.min(1, dx / (width() * ARM || 1))}>
  <svg class="blade" viewBox="0 0 40 12" aria-hidden="true"><path d="M2 6 H26 L38 2 Q40 6 38 10 L26 6" /><rect x="0" y="4.5" width="12" height="3" rx="1.5" /></svg>
  {#if armed}
    <button class="btn small slice" type="button" onclick={slice} in:dissolve={{ duration: 200 }}>Slice?</button>
  {/if}
</div>
<!-- svelte-ignore a11y_no_static_element_interactions -->
<div
  class="row"
  class:picked={selected}
  class:pending
  class:dragging
  aria-busy={pending}
  bind:this={rowEl}
  style:transform={dx ? `translateX(${dx}px)` : undefined}
  onpointerdown={down}
  onpointermove={move}
  onpointerup={up}
  onpointercancel={up}
  onclickcapture={swallow}
>
  <Check id={inputId} checked={selected} disabled={pending} onchange={onToggle} />
  <label class="text" for={inputId} title={[s.title, s.reason, aside].filter(Boolean).join(" · ")}>
    <span class="title">{s.title}</span>
    <span class="meta">{s.reason}{#if aside}{" · "}<span class:flag={!!s.note || !s.polite}>{aside}</span>{/if}</span>
  </label>
  <span class="mem" title={s.disk ? "Disk space" : "Memory"}>{s.disk ? `${bytes(s.disk)} disk` : bytes(s.memory)}</span>
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
    position: relative; /* above the knife */
    touch-action: pan-y;
  }
  .row.dragging {
    transition: none !important;
    cursor: grabbing;
  }
  .sliding .row {
    box-shadow: -8px 0 16px -10px rgba(74, 52, 16, 0.35);
    background: var(--surface);
    transition:
      transform 420ms var(--butter-ease),
      background-color var(--t-soft) var(--butter-ease);
  }
  .pat:not(.sliding) .row {
    transition:
      transform 420ms var(--butter-ease),
      background-color var(--t-soft) var(--butter-ease),
      opacity var(--t-soft) var(--butter-ease),
      filter var(--t-soft) var(--butter-ease);
  }
  /* The knife lives behind the row's left edge and shows as it slides. */
  .knife {
    position: absolute;
    inset: 0;
    display: flex;
    align-items: center;
    gap: 6px;
    padding-inline-start: 10px;
    border-radius: var(--r-pat);
    background: linear-gradient(90deg, var(--butter-hi), transparent 60%);
    opacity: var(--reveal, 0);
    pointer-events: none;
  }
  .knife.armed {
    pointer-events: auto;
  }
  .blade {
    width: 30px;
    height: 10px;
    flex: none;
    transform: translateX(calc((1 - var(--reveal, 0)) * -12px));
  }
  .blade path {
    fill: #d9d4c8;
    stroke: #9a927f;
    stroke-width: 0.6;
  }
  .blade rect {
    fill: var(--butter-deep);
  }
  .btn.slice {
    height: 26px;
    padding: 0 10px;
    font-size: 12px;
    font-weight: 600;
    border-color: var(--butter-edge);
    background: var(--surface);
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
