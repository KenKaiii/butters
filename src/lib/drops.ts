// Butter droplets with a little gravity: when a row melts, a few drops fall
// from its bottom edge to the footer puddle, squash on landing and vanish.
// Each drop is one small element animated by the Web Animations API on the
// compositor (transform/opacity), then removed. Capped so it never piles up.
import { prefersReducedMotion } from "svelte/motion";

const MAX_LIVE = 24;
let live = 0;

export function dropFrom(layer: HTMLElement, from: DOMRect, to: DOMRect, onLand?: () => void, count = 6) {
  if (prefersReducedMotion.current) {
    onLand?.();
    return;
  }
  const base = layer.getBoundingClientRect();
  let landed = false;
  for (let i = 0; i < count && live < MAX_LIVE; i++) {
    const el = document.createElement("i");
    el.className = "drop";
    const size = 4 + Math.random() * 4;
    el.style.width = `${size}px`;
    el.style.height = `${size * 1.25}px`;
    layer.appendChild(el);
    live++;

    const x = from.left - base.left + from.width * (0.12 + Math.random() * 0.76);
    const y0 = from.bottom - base.top - 4;
    // Land somewhere in the puddle, a bit closer to the middle.
    const xl = x + (to.left + to.width / 2 - base.left - x) * 0.25 + (Math.random() - 0.5) * 30;
    const y1 = to.top - base.top + 2;
    const fall = Math.max(40, y1 - y0);
    // t = sqrt(2d/g): taller falls take longer, like real drops.
    const dur = 260 + Math.sqrt(fall) * 26;
    const delay = 330 + i * 60 + Math.random() * 120; // start as the row's drips form

    const anim = el.animate(
      [
        { transform: `translate(${x}px, ${y0}px) scale(0.4, 0.6)`, opacity: 0, offset: 0 },
        { transform: `translate(${x}px, ${y0 + 3}px) scale(1, 1.3)`, opacity: 1, offset: 0.12 },
        { transform: `translate(${xl}px, ${y1}px) scale(0.9, 1.35)`, opacity: 1, offset: 0.86, easing: "ease-out" },
        { transform: `translate(${xl}px, ${y1 + 2}px) scale(1.9, 0.35)`, opacity: 0, offset: 1 },
      ],
      { duration: dur, delay, easing: "cubic-bezier(0.45, 0, 0.9, 0.5)", fill: "both" },
    );
    anim.finished
      .catch(() => {})
      .finally(() => {
        el.remove();
        live--;
        if (!landed) {
          landed = true;
          onLand?.();
        }
      });
  }
}
