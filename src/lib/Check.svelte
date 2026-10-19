<script lang="ts">
  // A soft checkbox (or switch) on top of the real input, so keyboard,
  // screen readers and form semantics stay native.
  let {
    checked,
    disabled = false,
    id,
    variant = "box",
    label,
    onchange,
  }: {
    checked: boolean;
    disabled?: boolean;
    id?: string;
    variant?: "box" | "switch";
    label?: string;
    onchange: (checked: boolean) => void;
  } = $props();
</script>

<span class="pick is-{variant}" class:on={checked} class:disabled>
  <input
    {id}
    type="checkbox"
    role={variant === "switch" ? "switch" : undefined}
    aria-label={label}
    {checked}
    {disabled}
    onchange={(e) => onchange(e.currentTarget.checked)}
  />
  {#if variant === "box"}
    <span class="box" aria-hidden="true">
      <svg viewBox="0 0 18 18"><path d="M4.5 9.5l3 3 6-6.5" /></svg>
    </span>
  {:else}
    <span class="track" aria-hidden="true"><span class="knob"></span></span>
  {/if}
</span>

<style>
  .pick {
    position: relative;
    display: inline-grid;
    place-items: center;
    flex: none;
    min-width: 24px;
    min-height: 24px;
  }
  input {
    position: absolute;
    inset: 0;
    margin: 0;
    opacity: 0;
    cursor: pointer;
    z-index: 1;
  }
  input:disabled {
    cursor: default;
  }

  .box {
    width: 18px;
    height: 18px;
    border-radius: 6px;
    border: 1.5px solid var(--control-line);
    background: var(--surface);
    display: grid;
    place-items: center;
    transition:
      background-color var(--t-soft) var(--butter-ease),
      border-color var(--t-soft) var(--butter-ease),
      transform var(--t-quick) var(--butter-ease);
  }
  .box svg {
    width: 14px;
    height: 14px;
    fill: none;
    stroke: var(--bg);
    stroke-width: 2.2;
    stroke-linecap: round;
    stroke-linejoin: round;
  }
  .box path {
    stroke-dasharray: 16;
    stroke-dashoffset: 16;
    transition: stroke-dashoffset var(--t-soft) var(--butter-ease);
  }
  .on .box {
    background: var(--ink);
    border-color: var(--ink);
  }
  .on .box path {
    stroke-dashoffset: 0;
    transition-delay: 60ms;
  }

  .track {
    width: 34px;
    height: 20px;
    border-radius: 10px;
    background: var(--track);
    border: 1.5px solid var(--control-line);
    position: relative;
    transition:
      background-color var(--t-soft) var(--butter-ease),
      border-color var(--t-soft) var(--butter-ease);
  }
  .knob {
    position: absolute;
    top: 2px;
    left: 2px;
    width: 13px;
    height: 13px;
    border-radius: 50%;
    background: var(--control-line);
    transition:
      transform var(--t-soft) var(--butter-ease),
      background-color var(--t-soft) var(--butter-ease);
  }
  .on .track {
    background: var(--ink);
    border-color: var(--ink);
  }
  .on .knob {
    background: var(--butter);
    transform: translateX(14px);
  }

  /* A soft squish when pressed. */
  .pick:active:not(.disabled) .box,
  .pick:active:not(.disabled) .knob {
    transform: scale(0.88);
  }
  .on.pick:active:not(.disabled) .knob {
    transform: translateX(14px) scale(0.88);
  }
  .disabled {
    opacity: 0.45;
  }

  input:focus-visible + .box,
  input:focus-visible + .track {
    outline: 2px solid var(--focus);
    outline-offset: 2px;
  }
  @media (prefers-reduced-motion: reduce) {
    .pick:active:not(.disabled) .box,
    .pick:active:not(.disabled) .knob {
      transform: none;
    }
    .on.pick:active:not(.disabled) .knob {
      transform: translateX(14px);
    }
  }
</style>
