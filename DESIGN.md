# Butters design

**Surface:** a small utility panel (menu bar / tray), 380 × 580.
**Single job:** see what's slowing the machine and close the right things in one action.
**Audience:** developers mid-work who glance at the panel, decide and leave.

## Thesis: butter, not yellow

Everything should feel soft, warm and smooth, the way butter behaves, not merely be the colour of it.

- **Colour.** Cream and milk neutrals with a warm cast. Always cream, even in system dark mode
  (the user's call: the dark variant read as "the old app"). Butter yellow
  is kept for three things only: the one primary button, the meters and the status drop.
  Danger is a muted terracotta. All tokens are in `src/app.css`.
- **Type.** Sora (variable, bundled, 33 KB, OFL), with tabular figures so live numbers don't
  jitter. Hierarchy: 16/600 heading, 13/600 item titles, 12.5/400 body, 11.5 labels.
- **Shape.** Soft radii (panel 16, rows 12, controls 10). Rows are borderless "pats"; a ticked
  row gets a pale cream fill. No hairline dividers inside the list.

## Motion: melting

One curve everywhere, `cubic-bezier(0.22, 1, 0.36, 1)`: quick to soften, long to settle.
Only opacity, filter and transform animate (`src/lib/motion.ts`).

| Moment | Motion |
|---|---|
| Panel opens | Pours in: drops 10 px, blur clears, settles (560 ms) |
| Panel closes | Slumps: sinks 6 px, squashes slightly, blurs out (220 ms, then the window is destroyed) |
| Rows appear | Pour in one after another on first fill (55 ms stagger) |
| Row closed | Melts: slumps, spreads a touch, sinks and blurs away; the rows below glide up |
| Text/state swaps | Dissolve: blur + fade in place (footer, heading, views) |
| Numbers | Glide to new values (900 ms) instead of jumping |
| Checkbox / switch | Fills softly, tick draws itself, slight squish on press |
| Status pat | Firm with a passing shine; softens with a drip; slumps, browns and drips under strain |
| Clicking away | The panel melts: content softens, it slumps and spreads, drips run off the bottom (520 ms) |
| Protected row | Memory dissolves into a Close button on hover; closing melts the row |
| Loading | Three cream pats with a slow sheen |

Reduced motion: every transition and keyframe has a `prefers-reduced-motion` path (JS
transitions return zero duration; CSS durations drop to 0).

## Icons

App icon: a melting butter pat with drips and a glossy puddle on a cream tile. Menu bar/tray:
the same pat in colour (not a macOS template image); under strain it becomes a browning, melting pat
with longer drips (no alarm dot). In the panel, the status icon is a tiny animated SVG pat
(`src/lib/StatusPat.svelte`): a shine glides across when smooth, one slow drip when busy, and it
slumps and drips when strained. CSS keyframes on transform/opacity only; still under reduced motion. Both drawn
by `scripts/make_icons.py`; run `cargo tauri icon icons/app-icon.png -o icons` afterwards.

## Checks

- Contrast: every text/control pair on the cream theme measured at or above WCAG 2.2 AA
  (text 4.5:1, focus ring and control borders 3:1).
- Focus: keyboard focus shows a 2 px ring; pointer clicks leave none (`:focus-visible`).
- Hit targets: at least 24 px.
