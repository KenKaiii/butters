<script lang="ts">
  // The "you saved" moment: big numbers, centred over the panel, counting
  // up like butter softening. Dismisses itself, or on click / Escape.
  // Deep clean uses it for disk: the same moment, plus a word about the
  // Trash when some of the space only comes back once it's emptied.
  import { onMount } from "svelte";
  import { Tween } from "svelte/motion";
  import { bytes } from "./format";
  import { butterEase, melt, ms, plop, pour } from "./motion";

  let {
    freed,
    closed,
    cpu,
    ports,
    onDone,
    total = 0,
    disk = false,
    trashed = 0,
    bin = "Trash",
  }: {
    freed: number;
    closed: number;
    cpu: number;
    ports: number;
    total?: number;
    onDone: () => void;
    /** Disk space (Deep clean) rather than memory. */
    disk?: boolean;
    /** Of `freed`, how much went to the Trash (comes back once emptied). */
    trashed?: number;
    /** "Trash" or "Recycle Bin". */
    bin?: string;
  } = $props();

  const amount = new Tween(0, { easing: butterEase });
  const GB = 1024 ** 3;
  const big = $derived(bytes(Math.max(0, amount.current)).split(" "));

  onMount(() => {
    amount.set(freed, { duration: ms(1100), delay: ms(180) });
    // A little longer when there's a Trash note to read.
    const t = setTimeout(onDone, (ms(3200) || 2600) + (trashed ? 1200 : 0));
    return () => clearTimeout(t);
  });

  const stats = $derived(
    [
      { n: `${closed}`, label: disk ? "cleaned" : "closed" },
      cpu >= 1 ? { n: `${Math.round(cpu)}%`, label: "CPU back" } : null,
      ports > 0 ? { n: `${ports}`, label: ports === 1 ? "port free" : "ports free" } : null,
    ].filter((x): x is { n: string; label: string } => !!x),
  );
</script>

<button class="scrim" type="button" aria-label="Dismiss" onclick={onDone} in:pour={{ y: 0, blur: 0, duration: 300 }} out:melt={{ duration: 520, y: 0, blur: 4 }}>
  <div class="osd" role="status" aria-live="polite" in:plop out:melt={{ duration: 640 }}>
    <span class="puddle" aria-hidden="true"></span>
    <p class="label">{disk ? "Cleaned up" : "Freed"}</p>
    <p class="big" aria-label={`${bytes(freed)} ${disk ? "cleaned up" : "freed"}`}>
      <span class="num">{big[0]}</span><span class="unit">{big[1]}</span>
    </p>
    <div class="stats">
      {#each stats as st, i (st.label)}
        <span class="stat" in:pour={{ delay: 380 + i * 90, y: 6 }}><strong>{st.n}</strong> {st.label}</span>
      {/each}
    </div>
    {#if trashed > 0}
      <p class="cheer" in:pour={{ delay: 640, y: 6 }}>Empty the {bin} to free {bytes(trashed)}</p>
    {:else if total > freed}
      <p class="cheer" in:pour={{ delay: 640, y: 6 }}>{bytes(total)} since install</p>
    {:else if freed >= GB}
      <p class="cheer" in:pour={{ delay: 640, y: 6 }}>Smooth as butter</p>
    {/if}
  </div>
</button>

<style>
  .scrim {
    position: absolute;
    inset: 0;
    z-index: 5;
    display: grid;
    place-items: center;
    border: 0;
    padding: 0;
    margin: 0;
    border-radius: inherit;
    background: rgba(250, 246, 236, 0.72);
    backdrop-filter: blur(6px);
    -webkit-backdrop-filter: blur(6px);
    color: var(--ink);
    cursor: pointer;
  }
  .osd {
    position: relative;
    display: flex;
    flex-direction: column;
    align-items: center;
    padding: 26px 34px 22px;
    border-radius: 28px;
    background: var(--surface);
    color: var(--ink);
    box-shadow: var(--shadow), 0 0 0 1px var(--frame);
  }
  /* A soft butter glow pooling under the number. */
  .puddle {
    position: absolute;
    left: 50%;
    top: 46%;
    width: 220px;
    height: 70px;
    transform: translate(-50%, -50%);
    border-radius: 50%;
    background: radial-gradient(closest-side, var(--butter-hi), transparent);
    opacity: 0.9;
    pointer-events: none;
  }
  .label,
  .big,
  .stats,
  .cheer {
    position: relative;
    margin: 0;
  }
  .label {
    font-size: 12px;
    font-weight: 600;
    color: var(--muted);
  }
  .big {
    display: flex;
    align-items: baseline;
    gap: 6px;
    line-height: 1;
    margin: 4px 0 12px;
  }
  .num {
    font-size: 64px;
    font-weight: 700;
    letter-spacing: -0.05em;
  }
  .unit {
    font-size: 22px;
    font-weight: 600;
    color: var(--muted);
    letter-spacing: -0.02em;
  }
  .stats {
    display: flex;
    gap: 14px;
    font-size: 12px;
    color: var(--muted);
    white-space: nowrap;
  }
  .stat strong {
    color: var(--ink);
    font-weight: 600;
  }
  .cheer {
    margin-top: 10px;
    font-size: 12px;
    font-weight: 600;
    color: var(--ink);
  }
</style>
