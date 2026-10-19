<script lang="ts" module>
  // Paths from Lucide (ISC licence, https://lucide.dev). Inlined so the panel
  // ships no icon font and makes no network requests.
  const line = (x1: number, y1: number, x2: number, y2: number) => `M${x1} ${y1}L${x2} ${y2}`;
  export const ICONS = {
    refresh: [
      "M3 12a9 9 0 0 1 9-9 9.75 9.75 0 0 1 6.74 2.74L21 8",
      "M21 3v5h-5",
      "M21 12a9 9 0 0 1-9 9 9.75 9.75 0 0 1-6.74-2.74L3 16",
      "M8 16H3v5",
    ],
    settings: [
      line(21, 4, 14, 4), line(10, 4, 3, 4), line(21, 12, 12, 12), line(8, 12, 3, 12),
      line(21, 20, 16, 20), line(12, 20, 3, 20), line(14, 2, 14, 6), line(8, 10, 8, 14), line(16, 18, 16, 22),
    ],
    eyeOff: [
      "M10.733 5.076a10.744 10.744 0 0 1 11.205 6.575 1 1 0 0 1 0 .696 10.747 10.747 0 0 1-1.444 2.49",
      "M14.084 14.158a3 3 0 0 1-4.242-4.242",
      "M17.479 17.499a10.75 10.75 0 0 1-15.417-5.151 1 1 0 0 1 0-.696 10.75 10.75 0 0 1 4.446-5.143",
      "m2 2 20 20",
    ],
    back: ["m12 19-7-7 7-7", "M19 12H5"],
    chevron: ["m9 18 6-6-6-6"],
    check: ["M20 6 9 17l-5-5"],
    chart: ["M3 3v16a2 2 0 0 0 2 2h16", "M18 17V9", "M13 17V5", "M8 17v-3"],
  } as const;
  export type IconName = keyof typeof ICONS;
</script>

<script lang="ts">
  let { name, size = 16 }: { name: IconName; size?: number } = $props();
</script>

<svg
  width={size}
  height={size}
  viewBox="0 0 24 24"
  fill="none"
  stroke="currentColor"
  stroke-width="2"
  stroke-linecap="round"
  stroke-linejoin="round"
  aria-hidden="true"
  focusable="false"
>
  {#each ICONS[name] as d (d)}<path {d} />{/each}
</svg>
