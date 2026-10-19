import { invoke } from "@tauri-apps/api/core";

export type Kind =
  | "agent_session"
  | "mcp_server"
  | "dev_server"
  | "watcher"
  | "language_server"
  | "local_model"
  | "browser"
  | "terminal"
  | "editor"
  | "container_runtime"
  | "app"
  | "process"
  | "docker_container"
  | "docker_disk"
  | "ollama_model";

export type Level = "smooth" | "busy" | "strained";

export interface Stats {
  memUsed: number;
  memTotal: number;
  swapUsed: number;
  cpu: number;
  cores: number;
  pressure: "normal" | "warn" | "critical" | "unknown";
  thermal: "nominal" | "fair" | "serious" | "critical" | "unknown";
}

export interface Suggestion {
  id: string;
  /** Stable across scans (ids change every scan). */
  key: string;
  kind: Kind;
  title: string;
  detail: string | null;
  reason: string;
  note: string | null;
  memory: number;
  cpu: number;
  ports: number[];
  recommended: boolean;
  polite: boolean;
  verb: string;
  /** Disk space it gives back (Docker clean-ups) instead of memory. */
  disk: number;
}

export interface Protected {
  /** Set when the user may close it anyway; null for locked items. */
  id: string | null;
  key: string;
  verb: string;
  title: string;
  kind: Kind;
  memory: number;
  cpu: number;
  reason: string;
}

export interface ScanResult {
  stats: Stats;
  level: Level;
  suggestions: Suggestion[];
  protected: Protected[];
  watchedSecs: number;
  scanMs: number;
}

export interface ServicesResult {
  suggestions: Suggestion[];
  docker: "running" | "off";
  ollama: "running" | "off";
}

export type Outcome =
  | { status: "closed"; message: string; freed: number; disk: number }
  | { status: "still_running"; message: string }
  | { status: "needs_force"; message: string }
  | { status: "refused"; message: string }
  | { status: "failed"; message: string };

export interface Settings {
  alerts: boolean;
  launch_at_login: boolean;
  sounds: boolean;
  shortcut: string;
  ignored: string[];
}

export interface UpdateStatus {
  current: string;
  available: string | null;
  checking: boolean;
  installing: boolean;
  error: string | null;
}

export interface Lifetime {
  /** Unix seconds of the first run. */
  since: number;
  freed: number;
  closed: number;
  by_kind: Record<string, number>;
  best_day: number;
  disk_freed: number;
}

export interface Culprit {
  label: string;
  avg: number;
  peak: number;
  series: number[];
}


export interface Smoothness {
  score: number | null;
  freedToday: number;
  freedWeek: number;
  week: number[];
  lifetime: Lifetime;
}

export type Sound = "open" | "tick" | "melt" | "freed" | "close";
export type Notify = "ask" | "off" | "on";

export const api = {
  scan: () => invoke<ScanResult>("scan"),
  smoothness: () => invoke<Smoothness>("smoothness"),
  fan: () => invoke<{ culprits: Culprit[]; minutes: number }>("fan_culprits"),
  sweepTargets: () => invoke<string[]>("sweep_targets"),
  live: () => invoke<{ stats: Stats; level: Level }>("live_stats"),
  /** Fire and forget; the backend checks the Sounds setting. */
  sound: (name: Sound) => void invoke("play_sound", { name }).catch(() => {}),
  notifyPermission: () => invoke<Notify>("notify_permission"),
  fixNotifyPermission: () => invoke<Notify>("fix_notify_permission"),
  scanServices: () => invoke<ServicesResult>("scan_services"),
  /** The last Docker/Ollama result, instantly; null before the first scan. */
  cachedServices: () => invoke<ServicesResult | null>("cached_services"),
  close: (id: string, force: boolean) => invoke<Outcome>("close_item", { id, force }),
  ignore: (id: string) => invoke<void>("ignore_item", { id }),
  getSettings: () => invoke<Settings>("get_settings"),
  setSettings: (settings: Settings) => invoke<Settings>("set_settings", { settings }),
  hide: () => invoke<void>("hide_panel"),
  /** Keep the panel open while closing things (quitting an app moves focus). */
  hold: (on: boolean) => void invoke("hold_panel", { on }).catch(() => {}),
  quit: () => invoke<void>("quit_butters"),
  updateStatus: () => invoke<UpdateStatus>("update_status"),
  checkUpdate: () => invoke<UpdateStatus>("check_update"),
  installUpdate: () => invoke<void>("install_update"),
};
