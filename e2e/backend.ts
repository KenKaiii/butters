// A fake Butters backend for end-to-end tests of the panel. It is installed
// into the page before the app boots and answers every command the panel
// sends, keeping simple state so tests can assert on what the panel did.
// The page reads it through window.__TAURI_INTERNALS__ like the real app.

export interface FakeState {
  calls: { cmd: string; args: unknown }[];
  sounds: string[];
  settings: { alerts: boolean; launch_at_login: boolean; sounds: boolean; shortcut: string; ignored: string[] };
  services: unknown[];
  sweep: string[];
  /** Simulated time Docker takes to answer a fresh check (ms). */
  servicesDelay: number;
  /** What the background check already has when the panel opens. */
  cachedServices: unknown[] | null;
  fan: { culprits: { label: string; avg: number; peak: number; series: number[] }[]; minutes: number };
  notify: "ask" | "off" | "on";
  update: { current: string; available: string | null; checking: boolean; installing: boolean; error: string | null };
  smoothness: {
    score: number | null;
    freedToday: number;
    freedWeek: number;
    week: number[];
    lifetime: { since: number; freed: number; closed: number; by_kind: Record<string, number>; best_day: number };
  };
  level: "smooth" | "busy" | "strained";
  /** Outcome per suggestion id for close_item (default: closed). */
  closeAs: Record<string, "closed" | "still_running" | "needs_force">;
}

/** Runs in the page (serialised by Playwright's addInitScript). */
export function installFakeBackend(init: Partial<FakeState> & { suggestions?: unknown[]; protectedItems?: unknown[] }) {
  const GB = 1024 ** 3;
  const MB = 1024 ** 2;
  const state = {
    calls: [] as { cmd: string; args: unknown }[],
    sounds: [] as string[],
    settings: { alerts: true, launch_at_login: false, sounds: true, shortcut: "Alt+Super+B", ignored: [] as string[] },
    services: [] as any[],
    sweep: ["a", "b"] as string[],
    servicesDelay: 0,
    cachedServices: null as unknown[] | null,
    fan: {
      culprits: [
        { label: "Google Chrome", avg: 62, peak: 140, series: [40, 55, 70, 140, 60, 50, 48, 66, 70, 61] },
        { label: "node", avg: 31, peak: 98, series: [10, 20, 98, 30, 25, 40, 22, 20, 33, 12] },
      ],
      minutes: 10,
    },
    notify: "on",
    update: { current: "0.1.0", available: null, checking: false, installing: false, error: null },
    closeAs: {} as Record<string, string>,
    smoothness: {
      score: 87,
      freedToday: 400 * MB,
      freedWeek: 3.2 * GB,
      week: [0, 1 * GB, 0, 512 * MB, 0, 1.3 * GB, 400 * MB],
      lifetime: {
        since: Math.floor(Date.now() / 1000) - 12 * 86_400,
        freed: 41.2 * GB,
        closed: 128,
        by_kind: { dev_server: 54, agent_session: 31, mcp_server: 22, browser: 9, docker_container: 7, app: 5 },
        best_day: 6.1 * GB,
      },
    },
    level: "smooth",
    ...init,
  };
  let memUsed = 18 * GB;
  let cpu = 30;
  const scan = {
    stats: { memUsed, memTotal: 24 * GB, swapUsed: 0, cpu, cores: 10, pressure: "normal", thermal: "nominal" },
    level: state.level,
    suggestions: (init.suggestions as unknown[]) ?? [
      { id: "a", key: "dev:vite", kind: "dev_server", title: "Vite dev server", detail: "~/app · :5173", reason: "Terminal closed", note: null, memory: 400 * MB, cpu: 0, ports: [5173], recommended: true, polite: true, verb: "Stop", disk: 0 },
      { id: "b", key: "agent:claude", kind: "agent_session", title: "Claude Code session", detail: "~/api", reason: "Idle 3 h", note: null, memory: 600 * MB, cpu: 4, ports: [], recommended: true, polite: true, verb: "Stop", disk: 0 },
      { id: "c", key: "app:chrome", kind: "browser", title: "Google Chrome", detail: null, reason: "Unused 2 h", note: null, memory: 2 * GB, cpu: 0, ports: [], recommended: false, polite: true, verb: "Quit", disk: 0 },
    ],
    protected: (init.protectedItems as any[]) ?? [
      { id: "x1-0", key: "app:ghostty#1", verb: "Quit", title: "Ghostty", kind: "terminal", memory: 300 * MB, cpu: 2, reason: "In use" },
    ],
    watchedSecs: 7200,
    scanMs: 400,
  };
  (window as any).__fake = state;
  // Real IPC sends JSON; copying the same way also unwraps Svelte's proxies.
  const copy = <T,>(v: T): T => JSON.parse(JSON.stringify(v));

  const handlers: Record<string, (args: any) => unknown> = {
    scan: () => copy(scan),
    scan_services: async () => {
      await new Promise((r) => setTimeout(r, state.servicesDelay));
      return { suggestions: copy(state.services), docker: state.services.length ? "running" : "off", ollama: "off" };
    },
    cached_services: () =>
      state.cachedServices ? { suggestions: copy(state.cachedServices), docker: "running", ollama: "off" } : null,
    sweep_targets: () => copy(state.sweep),
    fan_culprits: () => copy(state.fan),
    live_stats: () => {
      // Moves every call so tests can see the meters are live.
      memUsed -= 256 * MB;
      cpu = cpu === 30 ? 42 : 30;
      scan.stats.memUsed = memUsed;
      scan.stats.cpu = cpu;
      return { stats: copy(scan.stats), level: state.level };
    },
    close_item: ({ id, force }) => {
      const svc = (state.services as any[]).find((x) => x.id === id);
      if (svc) {
        state.services = state.services.filter((x: any) => x.id !== id);
        return { status: "closed", message: "Cleared", freed: svc.memory, disk: svc.disk ?? 0 };
      }
      const pi = (scan.protected as any[]).findIndex((x) => x.id === id);
      if (pi >= 0) {
        const [p] = scan.protected.splice(pi, 1) as any[];
        return { status: "closed", message: "Freed", freed: p.memory, disk: 0 };
      }
      const s = scan.suggestions.find((x: any) => x.id === id) as any;
      if (!s) return { status: "refused", message: "List changed. Refresh." };
      const as = force ? "closed" : (state.closeAs[id] ?? "closed");
      if (as === "closed") {
        scan.suggestions = scan.suggestions.filter((x: any) => x.id !== id);
        const lt = state.smoothness.lifetime;
        lt.freed += s.memory;
        lt.closed += 1;
        lt.by_kind[s.kind] = (lt.by_kind[s.kind] ?? 0) + 1;
        state.sweep = state.sweep.filter((x) => x !== id);
        return { status: "closed", message: "Freed", freed: s.memory, disk: 0 };
      }
      if (as === "needs_force") return { status: "needs_force", message: "No window, force close only" };
      return { status: "still_running", message: "Still open, may be asking to save" };
    },
    ignore_item: ({ id }) => {
      const s = scan.suggestions.find((x: any) => x.id === id) as any;
      state.settings.ignored.push(s.key);
      scan.suggestions = scan.suggestions.filter((x: any) => x.id !== id);
      return null;
    },
    get_settings: () => copy(state.settings),
    set_settings: ({ settings }) => {
      state.settings = copy(settings);
      return copy(state.settings);
    },
    play_sound: ({ name }) => {
      state.sounds.push(name);
      return null;
    },
    notify_permission: () => state.notify,
    smoothness: () => copy(state.smoothness),
    fix_notify_permission: () => {
      if (state.notify === "ask") state.notify = "on";
      return state.notify;
    },
    update_status: () => copy(state.update),
    check_update: () => copy(state.update),
    install_update: () => {
      state.update.installing = true;
      return null;
    },
    hide_panel: () => null,
    hold_panel: () => null,
    quit_butters: () => null,
  };

  (window as any).__TAURI_INTERNALS__ = {
    transformCallback: () => 0,
    invoke: async (cmd: string, args: any) => {
      state.calls.push({ cmd, args });
      const h = handlers[cmd];
      if (!h) throw new Error(`unknown command ${cmd}`);
      // Real IPC is async; a tiny delay keeps ordering realistic.
      await new Promise((r) => setTimeout(r, 15));
      return h(args ?? {});
    },
  };
}
