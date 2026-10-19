<script lang="ts">
  // Memory as a stick of butter that melts as RAM fills: the stick shortens,
  // a puddle spreads beneath it, drips start when it's tight.
  // CPU as a pat warming in a pan: it softens and shimmers as load rises.
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
  const heat = $derived(cpuPct / 100);
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
    <svg class="pan state-{cpuState}" viewBox="0 0 160 26" preserveAspectRatio="none" aria-hidden="true" style:--heat={heat}>
      <!-- the pan's heat: a warm glow that fills with load -->
      <rect class="pan-base" x="0" y="18" width="160" height="6" rx="3" />
      <rect class="pan-heat" x="0" y="18" width="160" height="6" rx="3" style:transform="scaleX({cpuPct / 100})" />
      <!-- a pat sliding across as it heats, softening on the way -->
      <g class="pat" style:transform="translateX({(cpuPct / 100) * 132}px)">
        <rect class="pat-body" x="0" y="7" width="26" height="11" rx="3" style:transform="scale({1 + heat * 0.18}, {1 - heat * 0.32})" />
        <rect class="pat-top" x="2" y="7" width="22" height="3.5" rx="1.75" style:transform="scale({1 + heat * 0.18}, {1 - heat * 0.32})" />
      </g>
      <!-- shimmer lines when it's working hard -->
      <g class="shimmer">
        <path d="M60 4 q3 -3 6 0 t6 0" />
        <path d="M90 6 q3 -3 6 0 t6 0" />
        <path d="M120 3 q3 -3 6 0 t6 0" />
      </g>
    </svg>
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

  /* cpu pan */
  .pan-base {
    fill: var(--track);
  }
  .pan-heat {
    fill: var(--butter);
    opacity: 0.6;
    transform-origin: 0 50%;
  }
  .pan.state-bad .pan-heat {
    fill: #e2a44a;
    opacity: 0.85;
  }
  .pat-body {
    fill: var(--butter);
    transform-origin: 50% 100%;
  }
  .pat-top {
    fill: var(--butter-hi);
    transform-origin: 50% 300%;
  }
  .pan.state-bad .pat-body {
    fill: #e8b453;
  }
  .shimmer path {
    fill: none;
    stroke: var(--butter-deep);
    stroke-width: 1.2;
    stroke-linecap: round;
    opacity: 0;
  }
  .pan.state-warn .shimmer path,
  .pan.state-bad .shimmer path {
    animation: g-shimmer 1.8s ease-in-out infinite;
  }
  .shimmer path:nth-child(2) {
    animation-delay: 0.5s !important;
  }
  .shimmer path:nth-child(3) {
    animation-delay: 1s !important;
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
  @keyframes g-shimmer {
    0% {
      opacity: 0;
      transform: translateY(2px);
    }
    40% {
      opacity: 0.6;
    }
    100% {
      opacity: 0;
      transform: translateY(-4px);
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .drip,
    .shimmer path {
      animation: none !important;
    }
  }
</style>
