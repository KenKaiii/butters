<script lang="ts">
  // Deep clean: developer leftovers on disk, one tab per kind. Same rows and
  // footer as the main list: tick, confirm, and each cleaned row melts away.
  // Looking and cleaning both run in the backend, so they carry on when the
  // panel closes or you go back; this view just watches. Reopened, it shows
  // where things got to: a look still going, a clean still going, or the
  // result of one that finished while you were away. A finished look is
  // reused for 10 minutes. Ticks are kept across tabs; the footer counts all.
  import { flip } from "svelte/animate";
  import { onMount } from "svelte";
  import { api, type DeepCategory, type DeepRow, type DeepStatus, type Outcome } from "./api";
  import { bytes } from "./format";
  import Icon from "./Icon.svelte";
  import Row from "./Row.svelte";
  import { butterEase, dissolve, melt, ms, pour } from "./motion";
  import { tip } from "./tip";

  let {
    days,
    bin = "Trash",
    onBack,
    onSay,
    onCleaned,
  }: {
    days: number;
    /** "Trash", or "Recycle Bin" on Windows. */
    bin?: string;
    onBack: () => void;
    onSay: (msg: string) => void;
    /** After a clean: the panel's big "Cleaned up" moment. */
    onCleaned: (r: { cleared: number; trashed: number; count: number }) => void;
  } = $props();

  let status = $state<DeepStatus | null>(null);
  let error = $state<string | null>(null);
  let overrides = $state<Record<string, boolean>>({});
  let melting = $state<Record<string, boolean>>({});
  // Rows the backend has cleaned stay on screen until their melt finishes.
  let meltingRows = $state<Record<string, DeepRow>>({});
  let gone = $state<Record<string, boolean>>({});
  let confirming = $state(false);
  // The confirm's Clean sits where "Clean N" was: it only works once it has
  // appeared, so a double-click can't skip the question.
  let armed = $state(false);
  let armTimer: ReturnType<typeof setTimeout> | undefined;
  function askFirst() {
    armed = false;
    confirming = true;
    clearTimeout(armTimer);
    // Not tied to motion: with reduced motion the footer swaps instantly,
    // and a double-click still needs to land on "ask", not "clean".
    armTimer = setTimeout(() => (armed = true), Math.max(ms(450), 400) + 50);
  }
  let poll: ReturnType<typeof setTimeout> | undefined;
  let shownClean = 0; // finishedAt of the clean whose pop-up we showed
  // False once the view is gone: a reply still on its way must not start
  // polling again or show anything. Every timer is tracked and cleared.
  let alive = true;
  const timers = new Set<ReturnType<typeof setTimeout>>();
  function later(fn: () => void, wait: number) {
    const t = setTimeout(() => {
      timers.delete(t);
      if (alive) fn();
    }, wait);
    timers.add(t);
  }

  const months = $derived(days >= 60 ? `${Math.round(days / 30)} months` : `${days} days`);
  const TABS: { id: DeepCategory; label: string; blurb: () => string }[] = [
    { id: "caches", label: "Caches", blurb: () => "Your tools download or rebuild these when needed" },
    { id: "projects", label: "Projects", blurb: () => `Build folders of projects you stopped working on. Ticked after ${months}` },
    { id: "mobile", label: "Mobile", blurb: () => "Simulators, old phone files, archives and emulators" },
    { id: "models", label: "Models", blurb: () => "Downloaded AI models. Slow to download again" },
  ];

  const rows = $derived(
    [...(status?.rows ?? []).filter((r) => !meltingRows[r.key]), ...Object.values(meltingRows)]
      .filter((r) => !gone[r.key])
      .sort((a, b) => Number(b.recommended) - Number(a.recommended) || b.disk - a.disk),
  );
  const tabs = $derived(TABS.map((t) => ({ ...t, rows: rows.filter((r) => r.category === t.id) })).filter((t) => t.rows.length));
  let picked = $state<DeepCategory | null>(null);
  // The chosen tab, or the first one with something in it.
  const tab = $derived(tabs.find((t) => t.id === picked) ?? tabs[0]);
  const tabEls: Record<string, HTMLButtonElement> = {};
  /** Arrow keys move between tabs, as in any tab list. */
  function tabKey(e: KeyboardEvent) {
    const i = tabs.findIndex((t) => t.id === tab?.id);
    const step = e.key === "ArrowRight" ? 1 : e.key === "ArrowLeft" ? -1 : 0;
    const to = e.key === "Home" ? 0 : e.key === "End" ? tabs.length - 1 : step ? (i + step + tabs.length) % tabs.length : -1;
    const next = tabs[to];
    if (!next) return;
    e.preventDefault();
    picked = next.id;
    tabEls[next.id]?.focus();
  }

  // Progress: the bar, what it's doing, and how much it has measured.
  const progress = $derived(status?.progress ?? null);
  const pct = $derived(Math.round(Math.min(1, Math.max(0, progress?.fraction ?? 0)) * 100));
  const stepText = $derived(progress ? (progress.total && progress.step !== "Measuring" ? `${progress.step} · ${progress.done} of ${progress.total}` : progress.step) : "Starting");
  const measureText = $derived(
    progress?.step === "Measuring" && progress.total
      ? `${progress.done} of ${progress.total} folders · ${bytes(progress.found)} so far`
      : status?.scannedAt == null
        ? "The first look can take a minute"
        : "",
  );
  // A clean in progress: which rows are queued, which one is going now.
  const cleaning = $derived(status?.cleaning ?? null);
  const running = $derived(!!cleaning);
  const pending = $derived(Object.fromEntries((cleaning?.keys ?? []).slice(cleaning?.done ?? 0).map((k) => [k, true])));
  const results = $derived((status?.outcomes ?? {}) as Record<string, Outcome>);
  const cleanPct = $derived(cleaning ? Math.round((cleaning.done / Math.max(1, cleaning.keys.length)) * 100) : 0);
  const currentTitle = $derived(cleaning?.current ? (status?.rows.find((r) => r.key === cleaning.current)?.title ?? "") : "");
  const lookedAgo = $derived(
    status?.scannedAt ? Math.max(0, Math.round((Date.now() / 1000 - status.scannedAt) / 60)) : null,
  );
  const isSelected = (r: DeepRow) => overrides[r.key] ?? r.recommended;
  const selected = $derived(rows.filter((r) => !melting[r.key] && isSelected(r)));
  const total = $derived(selected.reduce((n, r) => n + r.disk, 0));
  const deleted = $derived(selected.filter((r) => r.removal === "delete").reduce((n, r) => n + r.disk, 0));
  const trashed = $derived(selected.filter((r) => r.removal === "trash").reduce((n, r) => n + r.disk, 0));
  const upTo = $derived(selected.some((r) => r.upTo));
  const scanning = $derived(!status || status.scanning);

  const MELT_MS = 1000;

  /** Takes in a new status: melts rows the backend just cleaned, shows a
   *  finished clean once, and keeps watching while anything is going on. */
  function take(next: DeepStatus) {
    if (!alive) return;
    const before = status;
    status = next;
    error = null;
    const cleanedKeys = new Set([...(next.cleaning?.keys ?? []), ...(next.lastClean?.keys ?? [])]);
    const still = new Set(next.rows.map((r) => r.key));
    for (const r of before?.rows ?? []) {
      if (still.has(r.key) || !cleanedKeys.has(r.key) || meltingRows[r.key] || gone[r.key]) continue;
      meltingRows[r.key] = r;
      melting[r.key] = true;
      delete overrides[r.key];
      api.sound("melt");
      later(() => {
        gone[r.key] = true;
        delete melting[r.key];
        delete meltingRows[r.key];
      }, ms(MELT_MS));
    }
    const done = next.lastClean;
    if (done && !next.cleaning && done.finishedAt !== shownClean) {
      shownClean = done.finishedAt;
      // Marked seen right away, so it shows once even if the view closes
      // before the pop-up does. If that fails, it may show once more.
      void api.deepCleanSeen().catch(() => {});
      // Let the last row finish melting first.
      later(() => {
        if (done.count) {
          api.sound("freed");
          onCleaned({ cleared: done.cleared, trashed: done.trashed, count: done.count });
        } else {
          onSay("Nothing cleaned");
        }
      }, before?.cleaning ? ms(MELT_MS * 0.8) : 0);
    }
    clearTimeout(poll);
    if (next.scanning || next.cleaning) poll = setTimeout(() => alive && void load(), 400);
  }

  async function load(rescan = false) {
    clearTimeout(poll);
    try {
      take(await api.deepClean(rescan));
    } catch (e) {
      if (alive) error = String(e);
    }
  }

  function rescan() {
    gone = {};
    void load(true);
  }

  /** Hands the ticked rows to the backend, which cleans them one at a time
   *  (they all hit the same disk) and keeps going if the panel closes. */
  async function cleanSelected() {
    if (!armed) return;
    confirming = false;
    clearTimeout(poll);
    try {
      take(await api.deepCleanRun(selected.map((r) => r.id)));
    } catch (e) {
      onSay(String(e));
      void load();
    }
  }

  async function ignore(r: DeepRow) {
    try {
      await api.ignore(r.id);
      gone[r.key] = true;
      onSay(`Ignoring ${r.title}`);
    } catch (e) {
      onSay(String(e));
    }
  }

  // Escape backs out of the confirm before it leaves the view.
  function onKey(e: KeyboardEvent) {
    if (e.key === "Escape" && confirming) {
      e.stopPropagation();
      confirming = false;
    }
  }

  onMount(() => {
    void load();
    return () => {
      alive = false;
      clearTimeout(poll);
      clearTimeout(armTimer);
      for (const t of timers) clearTimeout(t);
      timers.clear();
    };
  });

  const footState = $derived(running ? "running" : confirming ? "confirm" : selected.length ? "ready" : "idle");
</script>

<svelte:window onkeydowncapture={onKey} />

<section class="view" aria-label="Deep clean" in:dissolve={{ duration: 220, blur: 3 }} out:dissolve={{ duration: 160, blur: 3 }}>
  <header class="head">
    <div class="status">
      <button class="ghost" type="button" aria-label="Back" use:tip={"Back"} onclick={onBack}><Icon name="back" /></button>
      <h1>Deep clean</h1>
    </div>
    <button
      class="ghost"
      type="button"
      aria-label="Look again"
      use:tip={{ text: "Look again", lines: lookedAgo === null ? [] : [lookedAgo < 1 ? "Looked just now" : `Looked ${lookedAgo} min ago`] }}
      onclick={rescan}
      disabled={!!status?.scanning || running}
    >
      <span class="spin" class:spinning={!!status?.scanning}><Icon name="refresh" /></span>
    </button>
  </header>

  {#if !scanning && tab && !error}
    <!-- svelte-ignore a11y_interactive_supports_focus -->
    <div class="tabs" role="tablist" aria-label="Kinds of leftovers" tabindex="-1" onkeydown={tabKey} in:dissolve={{ duration: 200, blur: 2 }}>
      {#each tabs as t (t.id)}
        {@const ticked = t.rows.filter((r) => isSelected(r)).length}
        <button
          type="button"
          role="tab"
          id={`deep-tab-${t.id}`}
          aria-selected={t.id === tab.id}
          aria-controls="deep-panel"
          tabindex={t.id === tab.id ? 0 : -1}
          class:on={t.id === tab.id}
          bind:this={tabEls[t.id]}
          use:tip={{ text: t.label, lines: [`${t.rows.length} found, ${ticked} ticked`, bytes(t.rows.reduce((n, r) => n + r.disk, 0))] }}
          onclick={() => (picked = t.id)}
        >
          {t.label}<span class="count">{t.rows.length}</span>
        </button>
      {/each}
    </div>
  {/if}

  <div class="scroll">
    {#if error}
      <div class="empty" in:pour>
        <p class="empty-title" use:tip={error}>Couldn't look at the disk</p>
        <button class="btn" type="button" onclick={() => load(true)}>Try again</button>
      </div>
    {:else if scanning && status?.supported !== false}
      <div class="looking" in:pour={{ y: -4 }}>
        <!-- One lasting live region (re-created ones often go unannounced);
             only the text inside it swaps. -->
        <p class="step" role="status">
          {#key progress?.step ?? "Starting"}<span in:dissolve={{ duration: 220, blur: 2 }}>{stepText}</span>{/key}
        </p>
        <div class="track" role="progressbar" aria-label="Deep clean progress" aria-valuemin={0} aria-valuemax={100} aria-valuenow={pct} aria-valuetext={`${pct}%, ${stepText}`}>
          <i class="fill" style:transform="scaleX({Math.max(pct, 2) / 100})"></i>
        </div>
        <p class="meta muted"><span>{measureText}</span><span class="pct">{pct}%</span></p>
      </div>
    {:else if status && !status.supported}
      <p class="empty muted" in:pour>Deep clean isn't available on this system</p>
    {:else if !rows.length}
      <div class="empty" in:pour>
        <p class="empty-title">Nothing to clean</p>
        <p class="muted">No big caches, idle simulators or forgotten build folders</p>
      </div>
    {:else if tab}
      <div role="tabpanel" id="deep-panel" aria-labelledby={`deep-tab-${tab.id}`}>
        {#key tab.id}
          <div in:dissolve={{ duration: 180, blur: 2 }}>
          <p class="blurb muted">{tab.blurb()}</p>
          <ul class="list">
            {#each tab.rows as r (r.key)}
              <li data-key={r.key} animate:flip={{ duration: ms(620), easing: butterEase }} out:melt={{ duration: gone[r.key] ? 0 : 760, blur: 0 }}>
                <Row
                  s={r}
                  selected={isSelected(r)}
                  melting={!!melting[r.key]}
                  pending={!!pending[r.key]}
                  result={results[r.key]}
                  onToggle={(on) => {
                    overrides[r.key] = on;
                    api.sound("tick");
                  }}
                  onIgnore={() => ignore(r)}
                  onForce={() => {}}
                  diskWord={false}
                />
              </li>
            {/each}
          </ul>
          </div>
        {/key}
      </div>
    {/if}
  </div>

  <footer class="bar">
    {#key footState}
      <div class="bar-inner" in:dissolve={{ delay: 60 }} out:dissolve={{ duration: 160 }}>
        {#if footState === "running" && cleaning}
          <div class="cleaning">
            <p class="bar-text two" role="status">
              <strong>Cleaning {Math.min(cleaning.done + 1, cleaning.keys.length)} of {cleaning.keys.length}{currentTitle ? ` · ${currentTitle}` : ""}</strong>
              <span class="muted">Keeps going if you close Butters</span>
            </p>
            <div class="track slim" role="progressbar" aria-label="Cleaning progress" aria-valuemin={0} aria-valuemax={100} aria-valuenow={cleanPct}>
              <i class="fill" style:transform="scaleX({Math.max(cleanPct, 3) / 100})"></i>
            </div>
          </div>
        {:else if footState === "confirm"}
          <!-- What happens, split by how: deleted for good vs. to the Trash. -->
          <p class="bar-text two">
            {#if deleted}<span><strong>{bytes(deleted)}</strong> deleted</span>{/if}
            {#if trashed}<span><strong>{bytes(trashed)}</strong> to the {bin}</span>{/if}
          </p>
          <div class="bar-actions">
            <button class="btn" type="button" onclick={() => (confirming = false)}>Cancel</button>
            <button class="btn primary" type="button" disabled={!armed} onclick={cleanSelected}>Clean {selected.length}</button>
          </div>
        {:else if footState === "ready"}
          <p class="bar-text">Frees {upTo ? "up to " : ""}<strong>{bytes(total)}</strong></p>
          <button class="btn primary" type="button" onclick={askFirst}>Clean {selected.length}</button>
        {:else}
          <p class="bar-text muted">{rows.length ? "Tick what to clean" : ""}</p>
        {/if}
      </div>
    {/key}
  </footer>
</section>

<style>
  .view {
    display: flex;
    flex-direction: column;
    min-height: 0;
    /* Never wider than the panel, however many tabs there are. */
    min-width: 0;
    width: 100%;
    height: 100%;
  }
  .head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 14px var(--pad) 10px;
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
  .ghost:hover:not(:disabled) {
    color: var(--ink);
    background: var(--surface-soft);
  }
  .ghost:disabled {
    cursor: default;
    opacity: 0.6;
  }
  .spin {
    display: grid;
  }
  .spinning {
    animation: spin 1100ms var(--butter-ease) infinite;
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
  .scroll {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    overscroll-behavior: contain;
    padding: 0 var(--inset) 16px;
    mask-image: linear-gradient(to bottom, transparent 0, #000 10px, #000 calc(100% - 18px), transparent 100%);
    scrollbar-width: thin;
  }
  /* Tabs: the same pills as the Stats view. */
  .tabs {
    display: flex;
    gap: 2px;
    min-width: 0;
    /* Pills carry their own inset, so their text lines up with the rows. */
    padding: 0 calc(var(--pad) - 6px) 8px;
    overflow-x: auto;
    scrollbar-width: none;
  }
  .tabs button {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    height: 28px;
    padding: 0 7px 0 9px;
    flex: none;
    border: 0;
    border-radius: 14px;
    background: transparent;
    color: var(--muted);
    font: inherit;
    font-size: 12px;
    font-weight: 600;
    line-height: 1.25;
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
  .count {
    font-weight: 500;
    opacity: 0.75;
  }
  .blurb {
    margin: 0;
    padding: 4px var(--inset) 8px;
    font-size: 11.5px;
  }
  /* Progress: a stick of butter filling its tray. */
  .looking {
    padding: 18px var(--inset) 10px;
  }
  .step {
    margin: 0 0 10px;
    font-size: 13px;
    font-weight: 600;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .track {
    position: relative;
    height: 10px;
    border-radius: 5px;
    background: var(--track);
    overflow: hidden;
  }
  .fill {
    position: absolute;
    inset: 0;
    border-radius: inherit;
    background: linear-gradient(90deg, var(--butter-hi), var(--butter) 70%, var(--butter-deep));
    transform-origin: 0 50%;
    transition: transform 420ms var(--butter-ease);
  }
  /* A slow sheen so the bar never looks stuck between updates. */
  .fill::after {
    content: "";
    position: absolute;
    inset: 0;
    background: linear-gradient(100deg, transparent 30%, rgba(255, 251, 238, 0.7) 50%, transparent 70%);
    background-size: 200% 100%;
    animation: bar-sheen 1400ms ease-in-out infinite;
  }
  @keyframes bar-sheen {
    from {
      background-position: 150% 0;
    }
    to {
      background-position: -50% 0;
    }
  }
  .meta {
    display: flex;
    justify-content: space-between;
    gap: 8px;
    margin: 8px 0 0;
    font-size: 11.5px;
  }
  .meta > span:first-child {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .cleaning {
    display: grid;
    gap: 8px;
    width: 100%;
    min-width: 0;
  }
  .track.slim {
    height: 6px;
    border-radius: 3px;
  }
  .pct {
    flex: none;
    font-weight: 600;
    color: var(--ink);
  }
  .list {
    position: relative;
    list-style: none;
    margin: 0;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .muted {
    color: var(--muted);
  }
  .empty {
    padding: 10px var(--inset);
  }
  .empty p {
    margin: 0 0 8px;
  }
  .empty-title {
    font-weight: 600;
  }
  .bar {
    position: relative;
    display: grid;
    /* Long text truncates instead of widening the panel. */
    grid-template-columns: minmax(0, 1fr);
    min-height: 58px;
    padding: 12px var(--pad);
    background: var(--surface);
    color: var(--ink);
  }
  .bar-inner {
    grid-area: 1 / 1;
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--gap);
  }
  .bar-text {
    margin: 0;
    min-width: 0;
    font-size: 12px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .bar-text strong {
    font-weight: 600;
  }
  .bar-text.two {
    display: flex;
    flex-direction: column;
    line-height: 1.35;
  }
  .bar-text.two > * {
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .bar-actions {
    display: flex;
    gap: 8px;
  }
  .btn {
    height: var(--control-h);
    padding: 0 14px;
    border: 1px solid var(--hairline);
    border-radius: var(--r-control);
    background: var(--surface);
    color: var(--ink);
    font-weight: 500;
    white-space: nowrap;
    cursor: pointer;
    transition:
      background-color var(--t-soft) var(--butter-ease),
      background-position var(--t-slow) var(--butter-ease),
      transform var(--t-quick) var(--butter-ease);
  }
  .btn:hover {
    background-color: var(--surface-soft);
    color: var(--ink);
  }
  .btn:active {
    transform: scale(0.97);
  }
  .btn:disabled {
    cursor: default;
  }
  /* The one butter button, as on the main list. */
  .btn.primary {
    border-color: var(--butter-edge);
    background-image: linear-gradient(180deg, var(--butter-hi) 0%, var(--butter) 50%, var(--butter-hi) 100%);
    background-size: 100% 200%;
    background-position: 0 0;
    color: #2e2820;
    font-weight: 600;
    box-shadow: inset 0 1px 0 rgba(255, 255, 255, 0.55), 0 1px 2px rgba(120, 86, 20, 0.18);
  }
  .btn.primary:hover {
    background-position: 0 100%;
    color: #2e2820;
  }
  @media (prefers-reduced-motion: reduce) {
    .spinning,
    .fill::after {
      animation: none;
    }
    .btn:active {
      transform: none;
    }
  }
</style>
