// Butters' tooltip: a small cream card instead of the system's grey box.
// One shared element for the whole panel, shown after a short pause on hover
// or keyboard focus, placed above (or below) the target and kept inside the
// window. Plain text only (textContent), one to a few short lines.
//
// Usage: <span use:tip={"Full title"}> or use:tip={{ text, onlyIfClipped: true }}
// `onlyIfClipped` shows it only when the element's text is cut off with an
// ellipsis, so lines that already fit don't sprout redundant tooltips.
import type { Action } from "svelte/action";

export type TipValue = string | null | undefined | { text: string | null | undefined; onlyIfClipped?: boolean; lines?: string[] };

const DELAY = 450;
const GAP = 6;
const EDGE = 8;

let el: HTMLDivElement | null = null;
let timer: ReturnType<typeof setTimeout> | undefined;
let owner: HTMLElement | null = null;

function card(): HTMLDivElement {
  if (el) return el;
  el = document.createElement("div");
  el.className = "butter-tip";
  el.setAttribute("role", "tooltip");
  el.id = "butter-tip";
  document.body.appendChild(el);
  return el;
}

function normalise(v: TipValue): { text: string; onlyIfClipped: boolean; lines: string[] } | null {
  if (!v) return null;
  if (typeof v === "string") return { text: v, onlyIfClipped: false, lines: [] };
  if (!v.text && !v.lines?.length) return null;
  return { text: v.text ?? "", onlyIfClipped: !!v.onlyIfClipped, lines: v.lines ?? [] };
}

/** True when any line of the element (or its ellipsed children) is cut off. */
function clipped(node: HTMLElement): boolean {
  const parts = [node, ...node.querySelectorAll<HTMLElement>("*")];
  return parts.some((n) => n.scrollWidth > n.clientWidth + 1 && getComputedStyle(n).textOverflow === "ellipsis");
}

function place(target: HTMLElement) {
  const c = card();
  const r = target.getBoundingClientRect();
  const w = c.offsetWidth;
  const h = c.offsetHeight;
  let x = r.left + r.width / 2 - w / 2;
  x = Math.max(EDGE, Math.min(x, window.innerWidth - w - EDGE));
  let y = r.top - h - GAP;
  let below = false;
  if (y < EDGE) {
    y = r.bottom + GAP;
    below = true;
  }
  c.style.transform = `translate(${Math.round(x)}px, ${Math.round(y)}px)`;
  c.dataset.side = below ? "below" : "above";
}

function show(target: HTMLElement, v: NonNullable<ReturnType<typeof normalise>>) {
  const c = card();
  c.replaceChildren();
  if (v.text) {
    const t = document.createElement("div");
    t.className = "butter-tip-title";
    t.textContent = v.text;
    c.appendChild(t);
  }
  for (const line of v.lines) {
    const l = document.createElement("div");
    l.className = "butter-tip-line";
    l.textContent = line;
    c.appendChild(l);
  }
  c.classList.remove("on");
  place(target);
  // next frame so the fade/settle plays from the placed position
  requestAnimationFrame(() => c.classList.add("on"));
  target.setAttribute("aria-describedby", c.id);
  owner = target;
}

export function hideTip() {
  clearTimeout(timer);
  el?.classList.remove("on");
  owner?.removeAttribute("aria-describedby");
  owner = null;
}

export const tip: Action<HTMLElement, TipValue> = (node, initial) => {
  let value = initial;
  const enter = () => {
    const v = normalise(value);
    if (!v || (v.onlyIfClipped && !clipped(node))) return;
    clearTimeout(timer);
    timer = setTimeout(() => show(node, v), DELAY);
  };
  const leave = () => {
    clearTimeout(timer);
    if (owner === node) hideTip();
  };
  const focus = () => {
    if (node.matches(":focus-visible")) enter();
  };
  const key = (e: KeyboardEvent) => e.key === "Escape" && owner === node && hideTip();
  node.addEventListener("pointerenter", enter);
  node.addEventListener("pointerleave", leave);
  node.addEventListener("pointerdown", leave);
  node.addEventListener("focus", focus);
  node.addEventListener("blur", leave);
  node.addEventListener("keydown", key);
  return {
    update(v) {
      value = v;
    },
    destroy() {
      leave();
      node.removeEventListener("pointerenter", enter);
      node.removeEventListener("pointerleave", leave);
      node.removeEventListener("pointerdown", leave);
      node.removeEventListener("focus", focus);
      node.removeEventListener("blur", leave);
      node.removeEventListener("keydown", key);
    },
  };
};

/** "a, b, c, d, e" -> first few items plus "+N more" (for long lists). */
export function shortList(csv: string | null | undefined, keep = 4): string[] {
  const items = (csv ?? "").split(/,\s*/).filter(Boolean);
  if (items.length <= keep + 1) return items;
  return [...items.slice(0, keep), `+${items.length - keep} more`];
}
