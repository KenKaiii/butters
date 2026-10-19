<script lang="ts">
  // The status icon: a tiny pat of butter that tells you how the machine feels.
  //   smooth   - a firm pat; a soft shine glides across now and then
  //   busy     - it softens: slight slump, one slow drip
  //   strained - it melts: slumps, puddle spreads, drips keep falling, browns
  // Pure SVG + CSS keyframes on transform/opacity only (compositor work, no
  // layout or JS timers). It only exists while the panel is open, and all
  // motion stops under reduced motion. Decorative: the heading says the state.
  let { level }: { level: "smooth" | "busy" | "strained" } = $props();
</script>

<svg class="status-pat level-{level}" viewBox="0 0 20 20" width="20" height="20" aria-hidden="true" focusable="false">
  <defs>
    <linearGradient id="pat-top" x1="0" y1="0" x2="0" y2="1">
      <stop offset="0" style="stop-color: var(--pat-hi)" />
      <stop offset="1" style="stop-color: var(--pat-top)" />
    </linearGradient>
    <linearGradient id="pat-front" x1="0" y1="0" x2="0" y2="1">
      <stop offset="0" style="stop-color: var(--pat-front)" />
      <stop offset="1" style="stop-color: var(--pat-low)" />
    </linearGradient>
    <clipPath id="pat-clip"><path d="M3 8.2 L6.6 5.6 H16.6 L13 8.2 Z M3 8.2 H13 V13 H3 Z M13 8.2 L16.6 5.6 V10.4 L13 13 Z" /></clipPath>
  </defs>

  <!-- puddle (centred under the pat; spreads as it melts) -->
  <ellipse class="puddle" cx="10" cy="14.6" rx="7.4" ry="1.5" />

  <g class="body">
    <!-- side, front, top faces of the pat -->
    <path class="side" d="M13 8.2 L16.6 5.6 V10.4 L13 13 Z" />
    <path class="front" d="M3 8.2 H13 V13 H3 Z" />
    <path class="top" d="M3 8.2 L6.6 5.6 H16.6 L13 8.2 Z" />
    <!-- shine that glides across the pat -->
    <g clip-path="url(#pat-clip)">
      <rect class="shine" x="-6" y="3" width="3" height="12" transform="skewX(-20)" />
    </g>
  </g>

  <!-- drips from the front edge -->
  <rect class="drip d1" x="5" y="12" width="1.8" height="4" rx="0.9" />
  <rect class="drip d2" x="9.6" y="12" width="1.5" height="3.2" rx="0.75" />
</svg>

<style>
  .status-pat {
    flex: none;
    display: block;
    overflow: visible;
    --pat-hi: #fff4cf;
    --pat-top: #f8e1a0;
    --pat-front: #f0cf78;
    --pat-low: #e6bd5c;
    --pat-side: #dcae4c;
    --pat-edge: rgba(150, 108, 24, 0.35);
  }
  .status-pat.level-busy {
    --pat-top: #f3d27e;
    --pat-front: #e8bd5a;
    --pat-low: #d9a640;
    --pat-side: #cf9a38;
  }
  .status-pat.level-strained {
    /* browning butter, not an alarm red */
    --pat-hi: #f6d58a;
    --pat-top: #e9b558;
    --pat-front: #d9963a;
    --pat-low: #c27a2a;
    --pat-side: #b46c24;
    --pat-edge: rgba(120, 66, 16, 0.45);
  }

  path,
  .puddle,
  .drip {
    transition:
      fill var(--t-slow) var(--butter-ease),
      transform var(--t-slow) var(--butter-ease),
      opacity var(--t-slow) var(--butter-ease);
  }
  .top {
    fill: url(#pat-top);
    stroke: var(--pat-edge);
    stroke-width: 0.5;
    stroke-linejoin: round;
  }
  .front {
    fill: url(#pat-front);
    stroke: var(--pat-edge);
    stroke-width: 0.5;
    stroke-linejoin: round;
  }
  .side {
    fill: var(--pat-side);
    stroke: var(--pat-edge);
    stroke-width: 0.5;
    stroke-linejoin: round;
  }
  .puddle {
    fill: var(--pat-front);
    opacity: 0.45;
    transform-box: fill-box;
    transform-origin: 50% 50%;
    transform: scaleX(0.75);
  }
  .body {
    transform-box: fill-box;
    transform-origin: 50% 100%;
  }
  .drip {
    fill: var(--pat-low);
    transform-box: fill-box;
    transform-origin: 50% 0;
    transform: scaleY(0);
    opacity: 0;
  }
  .shine {
    fill: #fffaf0;
    opacity: 0;
  }

  /* smooth: a firm pat, a shine glides across every few seconds */
  .level-smooth .shine {
    animation: shine 5.2s var(--butter-ease) 0.8s infinite;
  }

  /* busy: softening; one slow drip, the pat breathes a little */
  .level-busy .body {
    transform: scale(1.02, 0.95);
  }
  .level-busy .puddle {
    transform: scaleX(0.92);
    opacity: 0.55;
  }
  .level-busy .d1 {
    animation: drip 3.4s cubic-bezier(0.55, 0, 0.6, 1) infinite;
  }

  /* strained: melting; slumped, spreading puddle, drips keep falling */
  .level-strained .body {
    animation: slump 2.8s ease-in-out infinite;
  }
  .level-strained .puddle {
    animation: spread 2.8s ease-in-out infinite;
    opacity: 0.7;
  }
  .level-strained .d1 {
    animation: drip 1.9s cubic-bezier(0.55, 0, 0.6, 1) infinite;
  }
  .level-strained .d2 {
    animation: drip 1.9s cubic-bezier(0.55, 0, 0.6, 1) 0.8s infinite;
  }

  @keyframes shine {
    0% {
      transform: translateX(0);
      opacity: 0;
    }
    8% {
      opacity: 0.75;
    }
    30% {
      transform: translateX(24px);
      opacity: 0;
    }
    100% {
      transform: translateX(24px);
      opacity: 0;
    }
  }
  @keyframes drip {
    0% {
      transform: translateY(0) scaleY(0);
      opacity: 0;
    }
    15% {
      opacity: 1;
    }
    55% {
      transform: translateY(0) scaleY(1);
      opacity: 1;
    }
    85% {
      transform: translateY(3px) scaleY(0.6);
      opacity: 0;
    }
    100% {
      transform: translateY(3px) scaleY(0);
      opacity: 0;
    }
  }
  @keyframes slump {
    0%,
    100% {
      transform: scale(1.03, 0.92);
    }
    50% {
      transform: scale(1.06, 0.86);
    }
  }
  @keyframes spread {
    0%,
    100% {
      transform: scaleX(1);
    }
    50% {
      transform: scaleX(1.1);
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .status-pat * {
      animation: none !important;
    }
    .level-strained .d1,
    .level-busy .d1 {
      transform: scaleY(1);
      opacity: 1;
    }
  }
</style>
