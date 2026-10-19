// Butters' motion vocabulary. Things pour in (drop a little, blur clears,
// settle), dissolve in place, or melt out: they hold their shape for a
// moment, then slump, spread and sink before they soften away. Only
// opacity, filter and transform animate.
import { prefersReducedMotion } from "svelte/motion";
import { backOut, quintOut } from "svelte/easing";
import type { TransitionConfig } from "svelte/transition";

const still = () => prefersReducedMotion.current;
const clamp = (x: number) => Math.min(1, Math.max(0, x));

/** In: drips down a few pixels and comes into focus. */
export function pour(_node: Element, { delay = 0, duration = 560, y = -8, blur = 8 } = {}): TransitionConfig {
  if (still()) return { duration: 0 };
  return {
    delay,
    duration,
    easing: quintOut,
    css: (t, u) => `opacity:${t};filter:blur(${(u * blur).toFixed(2)}px);transform:translateY(${(u * y).toFixed(2)}px)`,
  };
}

/** In with a soft plop: lands slightly big, then settles like a dropped pat. */
export function plop(_node: Element, { delay = 0, duration = 700 } = {}): TransitionConfig {
  if (still()) return { duration: 0 };
  return {
    delay,
    duration,
    css: (t) => {
      const s = backOut(t);
      const fade = clamp(t * 2.5);
      return `opacity:${fade};filter:blur(${((1 - fade) * 6).toFixed(2)}px);transform:translateY(${((1 - s) * -14).toFixed(2)}px) scale(${(0.86 + 0.14 * s).toFixed(4)})`;
    },
  };
}

/** Out: keeps its shape briefly, then slumps from the bottom, spreads, sinks
 *  and softens. Visible until late, so it never just blinks out. */
export function melt(_node: Element, { duration = 760, y = 12, blur = 6 } = {}): TransitionConfig {
  if (still()) return { duration: 0 };
  return {
    duration,
    css: (t) => {
      const p = 1 - t; // 0 -> 1 over the outro
      const slump = clamp((p - 0.15) / 0.85) ** 2; // holds, then gives way
      const fade = clamp((p - 0.45) / 0.55); // stays solid for the first half
      return [
        `opacity:${(1 - fade).toFixed(3)}`,
        `filter:blur(${(slump * blur).toFixed(2)}px)`,
        `transform-origin:50% 100%`,
        `transform:translateY(${(slump * y).toFixed(2)}px) scale(${(1 + 0.06 * slump).toFixed(4)},${(1 - 0.7 * slump).toFixed(4)})`,
      ].join(";");
    },
  };
}

/** Swap in place: blur and fade only. */
export function dissolve(_node: Element, { delay = 0, duration = 380, blur = 6 } = {}): TransitionConfig {
  if (still()) return { duration: 0 };
  return {
    delay,
    duration,
    easing: quintOut,
    css: (t, u) => `opacity:${t};filter:blur(${(u * blur).toFixed(2)}px)`,
  };
}

/** Durations for JS-driven motion (number tweens, list reflow, timers). */
export const ms = (n: number) => (still() ? 0 : n);
export { quintOut as butterEase };
