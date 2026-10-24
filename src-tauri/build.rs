// Registers the app's own commands with Tauri's permission system, so the
// panel can only call what its capability file explicitly allows.
const COMMANDS: &[&str] = &[
    "scan",
    "scan_services",
    "cached_services",
    "live_stats",
    "smoothness",
    "fan_culprits",
    "sweep_targets",
    "deep_clean",
    "deep_clean_run",
    "deep_clean_seen",
    "play_sound",
    "notify_permission",
    "fix_notify_permission",
    "close_item",
    "ignore_item",
    "get_settings",
    "set_settings",
    "hide_panel",
    "hold_panel",
    "is_playground",
    "playground_reset",
    "playground_status",
    "quit_butters",
    "update_status",
    "check_update",
    "install_update",
];

fn main() {
    tauri_build::try_build(
        tauri_build::Attributes::new().app_manifest(tauri_build::AppManifest::new().commands(COMMANDS)),
    )
    .expect("failed to run tauri-build");
}
