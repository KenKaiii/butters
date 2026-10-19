<script lang="ts">
  // Toast mode: shown only when there's nothing to close and the machine is
  // smooth. A slice pops out of the toaster, a pat of butter drops on and
  // melts across it. Plays once per opening (CSS keyframes, then still).
  // The soft "plop" sound lands with the butter.
  import { onMount } from "svelte";
  import { api } from "./api";
  import { ms, pour } from "./motion";

  onMount(() => {
    const t = setTimeout(() => api.sound("open"), ms(1150) || 0);
    return () => clearTimeout(t);
  });
</script>

<div class="toast-mode" in:pour={{ delay: 120 }}>
  <svg viewBox="0 0 120 92" width="120" height="92" aria-hidden="true">
    <!-- the slice of toast (behind the toaster front) -->
    <g class="slice">
      <path class="crust" d="M38 46 V20 Q38 8 50 8 H70 Q82 8 82 20 V46 Z" />
      <path class="crumb" d="M42 46 V21 Q42 12 51 12 H69 Q78 12 78 21 V46 Z" />
      <!-- butter melting over it -->
      <g class="butter">
        <rect class="pat" x="52" y="12" width="16" height="9" rx="2.5" />
        <path class="spread" d="M46 22 Q60 17 74 22 Q76 30 70 31 Q66 39 62 31 Q57 36 54 30 Q46 31 46 22 Z" />
      </g>
    </g>
    <!-- toaster -->
    <rect class="toaster" x="22" y="40" width="76" height="40" rx="12" />
    <rect class="slot" x="36" y="40" width="48" height="5" rx="2.5" />
    <rect class="shine" x="30" y="48" width="22" height="3" rx="1.5" />
    <rect class="lever" x="98" y="52" width="8" height="4" rx="2" />
    <rect class="foot" x="30" y="80" width="10" height="4" rx="2" />
    <rect class="foot" x="80" y="80" width="10" height="4" rx="2" />
  </svg>
  <p class="line">All buttered up</p>
</div>

<style>
  .toast-mode {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 6px;
    padding: 18px 0 6px;
  }
  svg {
    overflow: visible;
  }
  .line {
    margin: 0;
    font-size: 13px;
    font-weight: 600;
    color: var(--ink);
  }
  * {
    transform-box: fill-box;
  }
  .toaster {
    fill: #efe6d3;
    stroke: #d8cbb0;
    stroke-width: 1;
  }
  .slot {
    fill: #cfc1a3;
  }
  .shine {
    fill: #fffaf0;
    opacity: 0.8;
  }
  .lever,
  .foot {
    fill: #b9a98a;
  }
  .crust {
    fill: #d39a52;
  }
  .crumb {
    fill: #f2d39a;
  }
  .pat {
    fill: var(--butter);
    stroke: var(--butter-edge);
    stroke-width: 0.6;
  }
  .spread {
    fill: var(--butter-hi);
    opacity: 0;
  }

  /* 1. pop: the slice springs up out of the slot */
  .slice {
    transform-origin: 50% 100%;
    animation: pop 620ms cubic-bezier(0.34, 1.56, 0.64, 1) 240ms both;
  }
  .lever {
    animation: lever 620ms var(--butter-ease) 200ms both;
  }
  /* 2. drop: the pat falls onto the toast */
  .pat {
    transform-origin: 50% 100%;
    animation:
      fall 420ms cubic-bezier(0.5, 0, 0.9, 0.5) 760ms both,
      soften 900ms var(--butter-ease) 1180ms both;
  }
  /* 3. melt: it spreads and drips down the slice */
  .spread {
    transform-origin: 50% 0;
    animation: spread 1100ms var(--butter-ease) 1250ms both;
  }

  @keyframes pop {
    from {
      transform: translateY(30px);
    }
    to {
      transform: translateY(0);
    }
  }
  @keyframes lever {
    0% {
      transform: translateY(14px);
    }
    100% {
      transform: translateY(0);
    }
  }
  @keyframes fall {
    from {
      transform: translateY(-46px) rotate(-12deg);
      opacity: 0;
    }
    30% {
      opacity: 1;
    }
    to {
      transform: translateY(0) rotate(0);
      opacity: 1;
    }
  }
  @keyframes soften {
    0% {
      transform: scale(1, 1);
    }
    15% {
      transform: scale(1.25, 0.7);
    }
    100% {
      transform: scale(1.1, 0.55) translateY(2px);
    }
  }
  @keyframes spread {
    from {
      opacity: 0;
      transform: scale(0.35, 0.2);
    }
    to {
      opacity: 0.95;
      transform: scale(1, 1);
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .slice,
    .lever,
    .pat,
    .spread {
      animation: none !important;
    }
    .spread {
      opacity: 0.95;
    }
  }
</style>
