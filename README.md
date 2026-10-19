# Butters

A menu bar (macOS) / tray (Windows) app that keeps a busy dev machine running smooth.
It finds what's slowing things down: leftover dev servers, idle coding-agent sessions,
orphaned MCP servers, idle Docker containers, Ollama models parked in memory, and heavy apps
you haven't touched in a while. You then pick what to close.

**It never closes anything on its own.**

## What it suggests

| Thing | When it's suggested | Pre-ticked? |
|---|---|---|
| Dev server, agent session, MCP server, watcher whose terminal is gone | Always (it's a leftover) | Yes |
| Dev server / agent session | Quiet for 30 min+ | Yes after 2 h |
| Language server, MCP server, watcher | Quiet for 1 h+ and heavy | No |
| Desktop app (browser, editor, Slack…) | Not in front for 1 h+ and over 800 MB | No |
| Docker Desktop with no containers running | Holding 500 MB+ | Yes |
| Docker container | Using under 1% CPU | No |
| Ollama model | Loaded in memory | Only if kept for 30 min+ more |
| Anything | Using 50%+ CPU | Command-line tools only |

"Quiet" is measured, not guessed: Butters notes each process's CPU time once a minute, and on
macOS it also reads when each terminal last had input or output.

## What it never touches

- **The app you're using**, plus anything you had in front in the last 15 minutes.
- Work running inside the app in front, like an editor's language servers. In a terminal,
  idle sessions can still be suggested, but never pre-ticked and always with a warning.
- **Butters itself**, and whatever started it (the terminal you ran it from).
- Terminals as a whole: quitting one would end every session in it.
- Anything owned by another user or the system, OS processes, and iOS Simulator apps.
- Anything you mark "leave alone" (the eye button). Clear the list in Settings.

## How closing works

1. Every target is checked again when you click: each process must still be the *same* one the
   scan saw (process id **and** start time, so a reused id can never hit a different program),
   owned by you, and not on the protected list. pids 0 and 1 are refused outright.
2. **Polite first.** Apps get the same Quit request as their own menu (macOS) or window close
   button (Windows), so they can ask you to save. Command-line tools get SIGTERM. Docker
   containers use `docker stop`; Ollama models use Ollama's own unload call.
3. **Force close** only appears if the polite way didn't work, and asks you to confirm first.

## Footprint

Measured on an M-series Mac (macOS 27), release build (before sounds and live meters were added):

- App bundle: 3.9 MB (includes the Sora font, 33 KB). Windows exe: 4 MB.
- Idle in the menu bar before the panel is first opened: **15 MB**, 0% CPU.
- The panel's web view is destroyed when it closes (after a 220 ms melt-away), so Butters drops back to about 30 MB.
  macOS keeps two small shared WebKit helpers (about 50 MB together) alive after the first open.
- Background work: one look at which app is in front every 3 s, and one light process sample a
  minute. A full scan takes about 0.6 s and only runs while the panel is open.

## Playground (try it safely)

`Butters Playground` is a separate test app with its own name, data and history. It starts
seven real throwaway processes in `~/.butters-playground` (Vite, Next.js and Storybook dev
servers left behind by a closed terminal, an idle Claude Code session, an MCP server, a busy
webpack watch and a process that leaks memory) and sees **only those**. Your real apps are
invisible to it. Close them as often as you like, then hit **Reset** to start them all again.
Quitting the Playground stops whatever is left. Storybook takes 2.5 s to quit on purpose, like
an app that saves first.

```sh
pnpm playground    # builds src-tauri/target/release/bundle/macos/Butters Playground.app
```

Needs Node.js. Never released or auto-updated; the real Butters doesn't contain any of it.

## Develop

Needs Rust (stable), Node 22+ and pnpm.

```sh
pnpm install
pnpm tauri dev                      # run with hot reload
cd src-tauri && cargo test          # engine, safety and real-process tests
pnpm test:e2e                       # panel end-to-end (WebKit + Chromium, fake backend)
pnpm tauri build --debug --bundles app --no-sign && python3 e2e/macos_app.py
                                    # real app on macOS: real clicks, real settings file, real process closed
cd src-tauri && cargo run --example scan   # dry run: prints what Butters would suggest
pnpm tauri build --bundles app,dmg  # macOS release (DMG window art: python3 scripts/make_dmg_background.py)
```

Windows from a Mac (compiles and links; tests run on Windows in CI):

```sh
cd src-tauri
PATH="$(brew --prefix llvm@20)/bin:$PATH" cargo xwin build --release --target x86_64-pc-windows-msvc
```

`.github/workflows/ci.yml` runs type checks, clippy and the tests on real macOS and Windows
machines, plus `cargo audit`.

## Layout

- `src-tauri/src/engine/`: pure rules (what a process is, grouping, protection, suggestions). No OS calls, fully unit tested.
- `src-tauri/src/actions.rs`: closing, with the re-check at click time.
- `src-tauri/src/platform/`: macOS/Windows bits (app in front, memory pressure, heat, polite quit).
- `src-tauri/src/services.rs`: Docker and Ollama.
- `src-tauri/src/sampler.rs`: process table and measured idle time.
- `src/`: the panel (Svelte 5, about 28 KB gzipped). Look and motion are described in `DESIGN.md`.

## Security notes

- The panel can call only Butters' own commands (`src-tauri/capabilities/panel.json`). There is
  no shell, file-system or HTTP plugin, and a strict Content Security Policy is in place.
- The panel never sends process ids. It refers to suggestions by an id from the last scan, and
  the backend keeps the real targets.
- External programs (`docker`) run with fixed arguments, never through a shell, with a timeout.
  Container ids and model names are checked against strict patterns first.
- Ollama is contacted only on `127.0.0.1:11434`, with no redirects and a size cap on replies.

## Permissions

Butters needs almost nothing:

- **Seeing and closing your own processes, and knowing which app is in front:** no permission on macOS or Windows.
- **Notifications** (alerts, "update ready"): the only one. If it's undecided or off, the panel shows a
  single "Turn on" row. One click shows the system prompt or opens the right Settings page.
- **Start at login:** a toggle in Settings. No system prompt.

## Updates and releases

Installed copies check GitHub Releases (`latest.json`) shortly after start and every 6 hours.
When an update is out, the panel shows "Update x.y.z ready" and Butters notifies you once. It
installs only when you click Update. Every update is signed, and Butters refuses any download whose
signature doesn't match the public key built into the app.

To ship a release:

```sh
scripts/release.sh 0.2.0      # bumps versions, tags v0.2.0, pushes
```

`.github/workflows/release.yml` then builds for macOS (Apple Silicon + Intel) and Windows, signs
and notarizes the Mac builds, and publishes the release with `latest.json`.

Repo secrets it uses: `TAURI_SIGNING_PRIVATE_KEY`, `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`,
`APPLE_CERTIFICATE`, `APPLE_CERTIFICATE_PASSWORD`, `APPLE_SIGNING_IDENTITY`, `APPLE_ID`,
`APPLE_PASSWORD`, `APPLE_TEAM_ID`. Keep the updater private key backed up somewhere safe: if it's
lost, installed copies can't accept new updates.

## Not done yet

- Windows builds aren't code-signed (that needs a separate Windows certificate), so SmartScreen
  may warn on first install. Updates are still signature-checked.
- On Windows, temperature and the OS memory-pressure signal aren't available to normal apps,
  so Butters uses memory used vs. total instead. Terminal-activity idle detection is macOS only.
