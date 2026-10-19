<script lang="ts">
  // Memory as a stick of butter that melts as RAM fills: the stick shortens,
  // a puddle spreads beneath it, drips start when it's tight.
  // CPU as butter sizzling in a pan: the fill is the value, bubbles rise and
  // pop in it, faster as load climbs.
  // SVG + transform/opacity only; numbers are tweened by the parent.
  import { bytes } from "./format";

  let {
    memUsed,
    memTotal,
    cpu,
    memState,
    cpuState,
  }: {
    memUsed: number;
    memTotal: number;
    cpu: number;
    memState: "ok" | "warn" | "bad";
    cpuState: "ok" | "warn" | "bad";
  } = $props();

  const memPct = $derived(memTotal ? Math.min(100, (memUsed / memTotal) * 100) : 0);
  // Free memory = the stick that's left. 100% used -> fully melted.
  const left = $derived(Math.max(0.06, 1 - memPct / 100));
  const cpuPct = $derived(Math.min(100, Math.max(0, cpu)));
  // Bubbles sit along the pan; each one only sizzles once the melted butter
  // (the CPU fill) has reached it, so they never appear past the value.
  const BUBBLES = [
    { at: 6, delay: 0, size: 3 },
    { at: 19, delay: 0.7, size: 4 },
    { at: 32, delay: 0.3, size: 3 },
    { at: 45, delay: 1.1, size: 4 },
    { at: 58, delay: 0.5, size: 3 },
    { at: 71, delay: 0.9, size: 4 },
    { at: 84, delay: 0.2, size: 3 },
  ];
</script>

<div class="gauges">
  <div class="gauge" role="meter" aria-label="Memory used" aria-valuemin={0} aria-valuemax={100} aria-valuenow={Math.round(memPct)} aria-valuetext={`${bytes(memUsed)} of ${bytes(memTotal)}`}>
    <div class="label"><span>Memory</span><span class="num">{memTotal ? `${bytes(memUsed)} of ${bytes(memTotal)}` : "…"}</span></div>
    <svg class="stick state-{memState}" viewBox="0 0 160 26" preserveAspectRatio="none" aria-hidden="true">
      <!-- the dish, then the melted puddle spreading across it as memory is used -->
      <rect class="dish" x="0" y="20" width="160" height="5" rx="2.5" />
      <rect class="puddle" x="0" y="20" width="160" height="5" rx="2.5" style:transform="scaleX({Math.max(0.04, memPct / 100)})" />
      <!-- the stick: what's still free; it shrinks from the right as it melts -->
      <g class="stick-body" style:transform="scaleX({left})">
        <rect class="wrapper" x="0" y="5" width="160" height="15" rx="3" />
        <rect class="top" x="0" y="5" width="160" height="5" rx="2.5" />
        <rect class="sheen" x="6" y="7" width="40" height="1.6" rx="0.8" />
      </g>
      <!-- drips run off the melting end when memory is tight -->
      <g class="drips" style:transform="translateX({left * 160 - 8}px)">
        <rect class="drip" x="0" y="17" width="3" height="7" rx="1.5" />
        <rect class="drip d2" x="5" y="17" width="2.4" height="5" rx="1.2" />
      </g>
    </svg>
  </div>

  <div class="gauge" role="meter" aria-label="CPU in use" aria-valuemin={0} aria-valuemax={100} aria-valuenow={Math.round(cpuPct)}>
    <div class="label"><span>CPU</span><span class="num">{Math.round(cpuPct)}%</span></div>
    <!-- Butter sizzling in a pan: the fill is melted butter (its length is
         the CPU use), bubbles rise and pop in it, faster as load climbs.
         Nothing slides or looks draggable; the bar alone carries the value. -->
    <div class="pan state-{cpuState}" aria-hidden="true">
      <span class="fill" style:--cpu={cpuPct / 100}></span>
      {#each BUBBLES as b (b.at)}
        <i class="bubble" class:on={cpuPct >= b.at + 4} style:left="{b.at}%" style:--s="{b.size}px" style:animation-delay="{b.delay}s"></i>
      {/each}
    </div>
  </div>
</div>

<style>
  .gauges {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 16px;
  }
  .label {
    display: flex;
    justify-content: space-between;
    gap: 8px;
    font-size: 11.5px;
    color: var(--muted);
    white-space: nowrap;
  }
  .label .num {
    color: var(--ink);
    font-weight: 600;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  svg {
    display: block;
    width: 100%;
    height: 26px;
    margin-top: 2px;
    overflow: visible;
  }
  g,
  rect {
    transform-box: fill-box;
    transition:
      transform 900ms var(--butter-ease),
      fill 900ms var(--butter-ease),
      opacity 600ms var(--butter-ease);
  }

  /* memory stick */
  .dish {
    fill: var(--track);
  }
  .puddle {
    fill: var(--butter);
    opacity: 0.75;
    transform-origin: 0 50%;
  }
  .stick-body {
    transform-origin: 0 50%;
  }
  .wrapper {
    fill: var(--butter);
  }
  .top {
    fill: var(--butter-hi);
  }
  .sheen {
    fill: #fffaf0;
    opacity: 0.8;
  }
  .stick.state-warn .wrapper {
    fill: #e9c262;
  }
  .stick.state-bad .wrapper {
    fill: var(--butter-deep);
  }
  .stick.state-bad .puddle {
    fill: var(--butter-deep);
    opacity: 0.85;
  }
  .drips {
    opacity: 0;
  }
  .stick.state-warn .drips,
  .stick.state-bad .drips {
    opacity: 1;
  }
  .drip {
    fill: var(--butter-deep);
    transform-origin: 50% 0;
    animation: g-drip 2.6s cubic-bezier(0.55, 0, 0.6, 1) infinite;
  }
  .drip.d2 {
    animation-delay: 1.1s;
  }
  .stick.state-ok .drip {
    animation: none;
  }

  /* cpu: sizzling butter. Same footprint as the memory dish (5 px tall,
     1 px from the bottom of a 26 px row) so the two gauges line up. */
  .pan {
    position: relative;
    height: 26px;
    margin-top: 2px;
  }
  .pan::before,
  .fill {
    content: "";
    position: absolute;
    left: 0;
    right: 0;
    bottom: 1px;
    height: 5px;
    border-radius: 2.5px;
  }
  .pan::before {
    background: var(--track);
  }
  .fill {
    background: linear-gradient(var(--butter-hi), var(--butter));
    /* clip instead of scale, so the rounded end stays round at any length */
    clip-path: inset(0 calc((1 - var(--cpu, 0)) * 100%) 0 0 round 2.5px);
  }
  .pan.state-warn .fill {
    background: linear-gradient(var(--butter), #e2b04c);
  }
  .pan.state-bad .fill {
    /* browning butter when the CPU is pinned */
    background: linear-gradient(#efc76a, #d4923a);
  }
  .bubble {
    position: absolute;
    bottom: 2px;
    width: var(--s);
    height: var(--s);
    margin-left: calc(var(--s) / -2);
    border-radius: 50%;
    background: rgba(255, 250, 235, 0.6);
    box-shadow: inset 0 0 0 1px var(--butter-deep);
    opacity: 0;
    --dur: 2.6s;
  }
  .pan.state-warn .bubble {
    --dur: 1.5s;
  }
  .pan.state-bad .bubble {
    --dur: 0.9s;
  }
  .bubble.on {
    animation: sizzle var(--dur) ease-out infinite;
  }
  @keyframes sizzle {
    0% {
      transform: translateY(2px) scale(0.4);
      opacity: 0;
    }
    25% {
      opacity: 1;
    }
    80% {
      transform: translateY(-8px) scale(1);
      opacity: 0.9;
    }
    100% {
      transform: translateY(-10px) scale(1.4);
      opacity: 0;
    }
  }

  @keyframes g-drip {
    0% {
      transform: scaleY(0);
      opacity: 0;
    }
    20% {
      opacity: 1;
    }
    60% {
      transform: scaleY(1);
    }
    100% {
      transform: translateY(5px) scaleY(0.4);
      opacity: 0;
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .drip {
      animation: none !important;
    }
    .bubble {
      display: none;
    }
  }
</style>
