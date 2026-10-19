<script lang="ts">
  // "Butters so far": everything freed since install. The big number counts
  // up like butter softening; the per-kind bars pour in one after another.
  import { onMount } from "svelte";
  import { Tween } from "svelte/motion";
  import { api, type Culprit, type Smoothness } from "./api";
  import { bytes } from "./format";
  import Icon from "./Icon.svelte";
  import { butterEase, ms, pour } from "./motion";

  let { smooth, onBack, tab: initialTab = "life" }: { smooth: Smoothness | null; onBack: () => void; tab?: "life" | "fan" } = $props();

  // svelte-ignore state_referenced_locally
  let tab = $state<"life" | "fan">(initialTab);
  let fan = $state<{ culprits: Culprit[]; minutes: number } | null>(null);
  async function loadFan() {
    fan = await api.fan().catch(() => fan);
  }
  $effect(() => {
    if (tab !== "fan") return;
    void loadFan();
    const t = setInterval(loadFan, 30_000);
    return () => clearInterval(t);
  });
  const fanMax = $derived(Math.max(100, ...(fan?.culprits.flatMap((c) => c.series) ?? [0])));
  /** A smooth sparkline path through the per-minute values. */
  function spark(series: number[], w: number, h: number) {
    if (!series.length) return "";
    const step = series.length > 1 ? w / (series.length - 1) : w;
    const pts = series.map((v, i) => [i * step, h - (Math.min(v, fanMax) / fanMax) * (h - 2) - 1]);
    return pts.map(([x, y], i) => `${i ? "L" : "M"}${x.toFixed(1)} ${y.toFixed(1)}`).join(" ");
  }

  const KIND: Record<string, string> = {
    dev_server: "Dev servers",
    agent_session: "Agent sessions",
    mcp_server: "MCP servers",
    watcher: "Watchers",
    language_server: "Language servers",
    local_model: "Local models",
    ollama_model: "Ollama models",
    docker_disk: "Docker clean-ups",
    docker_container: "Containers",
    container_runtime: "Docker",
    browser: "Browsers",
    editor: "Editors",
    terminal: "Terminals",
    app: "Apps",
    process: "Other",
  };

  const life = $derived(smooth?.lifetime);
  const total = new Tween(0, { easing: butterEase });
  const big = $derived(bytes(Math.max(0, total.current)).split(" "));
  const kinds = $derived(
    Object.entries(life?.by_kind ?? {})
      .sort((a, b) => b[1] - a[1])
      .slice(0, 6)
      .map(([k, n]) => ({ label: KIND[k] ?? "Other", n })),
  );
  const most = $derived(Math.max(1, ...kinds.map((k) => k.n)));
  const since = $derived(life?.since ? new Date(life.since * 1000) : null);
  const days = $derived(since ? Math.max(1, Math.floor((Date.now() - since.getTime()) / 86_400_000)) : 0);

  $effect(() => {
    total.set(life?.freed ?? 0, { duration: ms(1100), delay: ms(150) });
  });
  onMount(() => () => total.set(0, { duration: 0 }));
</script>

<section class="view" aria-label="Stats" in:pour={{ y: -6 }}>
  <header class="head">
    <div class="status">
      <button class="ghost" type="button" aria-label="Back" title="Back" onclick={onBack}><Icon name="back" /></button>
      <h1>{tab === "life" ? "Butters so far" : "What's making the fan spin"}</h1>
    </div>
  </header>
  <div class="tabs" role="tablist" aria-label="Stats">
    <button type="button" role="tab" aria-selected={tab === "life"} class:on={tab === "life"} onclick={() => (tab = "life")}>So far</button>
    <button type="button" role="tab" aria-selected={tab === "fan"} class:on={tab === "fan"} onclick={() => (tab = "fan")}>Fan</button>
  </div>

  <div class="scroll stats">
    {#if tab === "fan"}
      {#if !fan}
        <p class="empty muted">Reading…</p>
      {:else if !fan.culprits.length}
        <p class="empty muted" in:pour>All quiet{fan.minutes < 5 ? " so far" : ""}</p>
      {:else}
        <p class="window muted" in:pour>Last {fan.minutes} min, average CPU</p>
        <ul class="fan">
          {#each fan.culprits as c, i (c.label)}
            <li in:pour={{ delay: 40 + i * 50 }} title={`${c.label}: avg ${Math.round(c.avg)}%, peak ${Math.round(c.peak)}% (100% = one core)`}>
              <span class="f-label">{c.label}</span>
              <svg class="f-spark" viewBox="0 0 90 18" preserveAspectRatio="none" aria-hidden="true">
                <path d={spark(c.series, 90, 18)} />
              </svg>
              <span class="f-n">{Math.round(c.avg)}%</span>
            </li>
          {/each}
        </ul>
      {/if}
    {:else if !life || life.closed === 0}
      <p class="empty muted" in:pour>Nothing closed yet</p>
    {:else}
      <div class="hero" in:pour={{ delay: 40 }}>
        <p class="label">Freed in total</p>
        <p class="big" aria-label={`${bytes(life.freed)} freed in total`}>
          <span class="num">{big[0]}</span><span class="unit">{big[1]}</span>
        </p>
        {#if since}
          <p class="since muted" title={since.toLocaleString()}>
            Since {since.toLocaleDateString(undefined, { day: "numeric", month: "short", year: "numeric" })} · {days} day{days === 1 ? "" : "s"}
          </p>
        {/if}
      </div>

      <div class="grid" in:pour={{ delay: 120 }}>
        <div class="cell"><span class="muted">Closed</span><strong>{life.closed}</strong></div>
        <div class="cell"><span class="muted">Best day</span><strong>{bytes(life.best_day)}</strong></div>
        <div class="cell"><span class="muted">This week</span><strong>{bytes(smooth?.freedWeek ?? 0)}</strong></div>
        <div class="cell"><span class="muted">{life.disk_freed ? "Disk cleared" : "Today"}</span><strong>{life.disk_freed ? bytes(life.disk_freed) : smooth?.score != null ? `Buttery ${smooth.score}` : "–"}</strong></div>
      </div>

      {#if kinds.length}
        <h2 class="section" in:pour={{ delay: 180 }}>What it closed</h2>
        <ul class="kinds">
          {#each kinds as k, i (k.label)}
            <li in:pour={{ delay: 220 + i * 50 }}>
              <span class="k-label">{k.label}</span>
              <span class="k-bar" aria-hidden="true"><i style:--w={k.n / most} style:animation-delay="{280 + i * 60}ms"></i></span>
              <span class="k-n">{k.n}</span>
            </li>
          {/each}
        </ul>
      {/if}
    {/if}
  </div>
</section>

<style>
  .view {
    position: relative; /* above the panel texture */
    z-index: 1;
    display: grid;
    grid-template-rows: auto 1fr;
    min-height: 0;
    height: 100%;
  }
  .head {
    display: flex;
    align-items: center;
    padding: 14px var(--pad) 6px;
  }
  .view {
    grid-template-rows: auto auto 1fr !important;
  }
  .tabs {
    display: flex;
    gap: 4px;
    padding: 0 var(--pad) 8px;
  }
  .tabs button {
    height: 28px;
    padding: 0 12px;
    border: 0;
    border-radius: 14px;
    background: transparent;
    color: var(--muted);
    font: inherit;
    font-size: 12px;
    font-weight: 600;
    cursor: pointer;
    transition:
      background-color var(--t-soft) var(--butter-ease),
      color var(--t-soft) var(--butter-ease);
  }
  .tabs button:hover {
    background: var(--surface-soft);
    color: var(--ink);
  }
  .tabs button.on {
    background: var(--ink);
    color: var(--bg);
  }
  .window {
    margin: 0;
    padding: 4px var(--inset) 6px;
    font-size: 12px;
  }
  .fan {
    list-style: none;
    margin: 0;
    padding: 0;
  }
  .fan li {
    display: grid;
    grid-template-columns: minmax(0, 1fr) 90px 44px;
    align-items: center;
    gap: 10px;
    padding: 8px var(--inset);
    border-radius: var(--r-pat);
    font-size: 13px;
  }
  .fan li:hover {
    background: var(--surface-soft);
  }
  .f-label {
    font-weight: 600;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .f-spark {
    width: 90px;
    height: 18px;
    overflow: visible;
  }
  .f-spark path {
    fill: none;
    stroke: var(--butter-deep);
    stroke-width: 1.6;
    stroke-linejoin: round;
    stroke-linecap: round;
    vector-effect: non-scaling-stroke;
  }
  .f-n {
    text-align: end;
    font-weight: 600;
  }
  .status {
    display: flex;
    align-items: center;
    gap: 10px;
    min-width: 0;
  }
  h1 {
    margin: 0;
    font-size: 16px;
    font-weight: 600;
    letter-spacing: -0.025em;
    line-height: 1.3;
  }
  .ghost {
    width: 30px;
    height: 30px;
    display: grid;
    place-items: center;
    border: 0;
    border-radius: var(--r-control);
    background: transparent;
    color: var(--muted);
    cursor: pointer;
    transition:
      color var(--t-soft) var(--butter-ease),
      background-color var(--t-soft) var(--butter-ease);
  }
  .ghost:hover {
    color: var(--ink);
    background: var(--surface-soft);
  }
  .scroll {
    overflow-y: auto;
    padding: 0 var(--pad) 16px;
    min-height: 0;
  }
  .muted {
    color: var(--muted);
  }
  .empty {
    margin: 0;
    padding: 10px var(--inset);
  }
  .hero {
    padding: 6px var(--inset) 14px;
  }
  .label,
  .since {
    margin: 0;
    font-size: 12px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .label {
    font-weight: 600;
    color: var(--muted);
  }
  .big {
    display: flex;
    align-items: baseline;
    gap: 6px;
    margin: 2px 0 4px;
    line-height: 1;
  }
  .num {
    font-size: 56px;
    font-weight: 700;
    letter-spacing: -0.05em;
  }
  .unit {
    font-size: 20px;
    font-weight: 600;
    color: var(--muted);
  }
  .grid {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 6px;
    padding: 0 0 6px;
  }
  .cell {
    display: flex;
    flex-direction: column;
    gap: 1px;
    padding: 10px var(--inset);
    border-radius: var(--r-pat);
    background: var(--surface-soft);
    color: var(--ink);
    font-size: 12px;
    min-width: 0;
  }
  .cell strong {
    font-size: 15px;
    font-weight: 600;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .section {
    margin: 0;
    padding: 12px var(--inset) 6px;
    font-size: 11.5px;
    font-weight: 600;
    color: var(--muted);
  }
  .kinds {
    list-style: none;
    margin: 0;
    padding: 0;
  }
  .kinds li {
    display: grid;
    grid-template-columns: minmax(0, 120px) 1fr auto;
    align-items: center;
    gap: 10px;
    padding: 6px var(--inset);
    font-size: 12.5px;
  }
  .k-label {
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .k-n {
    font-weight: 600;
    min-width: 2ch;
    text-align: end;
  }
  .k-bar {
    height: 6px;
    border-radius: 3px;
    background: var(--track);
    overflow: hidden;
  }
  .k-bar i {
    display: block;
    height: 100%;
    border-radius: 3px;
    background: linear-gradient(90deg, var(--butter-hi), var(--butter));
    transform-origin: 0 50%;
    transform: scaleX(var(--w));
    animation: k-pour 900ms var(--butter-ease) both;
  }
  @keyframes k-pour {
    from {
      transform: scaleX(0);
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .k-bar i {
      animation: none;
    }
  }
</style>
