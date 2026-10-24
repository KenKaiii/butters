pub mod actions;
pub mod alerts;
pub mod app;
pub mod deepclean;
pub mod docker_activity;
pub mod engine;
pub mod history;
pub mod log;
pub mod panel;
pub mod permissions;
pub mod platform;
#[cfg(feature = "playground")]
pub mod playground;
pub mod sampler;
pub mod services;
pub mod settings;
pub mod trends;
pub mod sound;
pub mod updates;

use tauri::Manager;

/// The Playground's test processes (Playground builds only).
#[cfg(feature = "playground")]
pub fn playground_state() -> &'static playground::Playground {
    static P: std::sync::OnceLock<playground::Playground> = std::sync::OnceLock::new();
    P.get_or_init(playground::Playground::default)
}

pub fn run() {
    tauri::Builder::default()
        // Must be first: a second launch just opens the existing panel.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| panel::open(app, None)))
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_autostart::Builder::new().build())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, _shortcut, event| {
                    if event.state == tauri_plugin_global_shortcut::ShortcutState::Pressed {
                        panel::toggle_from_shortcut(app);
                    }
                })
                .build(),
        )
        .manage(updates::UpdateState::default())
        .setup(|app| {
            // Menu bar app: no Dock icon, no app switcher entry. LSUIElement in
            // Info.plist keeps it that way when macOS "opens" the running app
            // (notification clicks); this covers unbundled `cargo run`.
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);
            let log = app.path().app_log_dir().map(|d| log::Log::open(&d)).unwrap_or_else(|_| log::Log::disabled());
            log_panics(&log);
            log.info(format!(
                "Butters {} started ({} {}, pid {})",
                app.package_info().version,
                std::env::consts::OS,
                std::env::consts::ARCH,
                std::process::id()
            ));
            let config_dir = app.path().app_config_dir()?;
            app.manage(app::AppState::new(settings::Settings::path(&config_dir), log));
            panel::setup_tray(app.handle())?;
            permissions::setup(app.handle());
            // The Playground leaves the global shortcut to the real Butters.
            #[cfg(not(feature = "playground"))]
            app::apply_shortcut(app.handle(), "");
            app::spawn_background(app.handle().clone());
            // The Playground never updates itself or touches real processes;
            // it starts its own set of test processes instead.
            #[cfg(not(feature = "playground"))]
            updates::spawn_checks(app.handle().clone());
            #[cfg(feature = "playground")]
            std::thread::spawn(|| {
                let _ = playground_state().reset();
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            app::scan,
            app::scan_services,
            app::cached_services,
            app::live_stats,
            app::smoothness,
            app::fan_culprits,
            app::sweep_targets,
            app::deep_clean,
            app::deep_clean_run,
            app::deep_clean_seen,
            sound::play_sound,
            permissions::notify_permission,
            permissions::fix_notify_permission,
            app::close_item,
            app::ignore_item,
            app::get_settings,
            app::set_settings,
            app::hide_panel,
            app::hold_panel,
            app::quit_butters,
            updates::update_status,
            updates::check_update,
            updates::install_update,
            app::is_playground,
            #[cfg(feature = "playground")]
            app::playground_reset,
            #[cfg(feature = "playground")]
            app::playground_status,
        ])
        .build(tauri::generate_context!())
        .expect("error while starting Butters")
        .run(|app, event| match event {
            // Closing the panel must not quit the app; only "Quit Butters" does.
            tauri::RunEvent::ExitRequested { code: None, api, .. } => api.prevent_exit(),
            tauri::RunEvent::Exit => {
                if let Some(state) = app.try_state::<app::AppState>() {
                    state.log.info("Butters exited");
                }
                // The Playground cleans up every test process it started.
                #[cfg(feature = "playground")]
                playground_state().stop_all();
            }
            _ => {}
        });
}

/// Writes a crash's message and location to the log before the default
/// handler runs (release builds abort right after).
fn log_panics(log: &log::Log) {
    let log = log.clone();
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let at = info.location().map(|l| format!(" at {}:{}", l.file(), l.line())).unwrap_or_default();
        let msg = info
            .payload()
            .downcast_ref::<&str>()
            .map(|s| (*s).to_string())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "unknown".into());
        log.error(format!("Crashed: {msg}{at}"));
        default(info);
    }));
}

#[cfg(test)]
mod tests {
    /// Without LSUIElement, a click on a Butters notification makes macOS
    /// promote the running app to a regular Dock app.
    #[test]
    fn bundle_is_a_menu_bar_app() {
        let plist: String = include_str!("../Info.plist").split_whitespace().collect();
        assert!(plist.contains("<key>LSUIElement</key><true/>"), "Info.plist must set LSUIElement to true");
    }
}

#[cfg(test)]
mod permission_tests {
    /// Every command the app registers must also be on the panel's
    /// allow-list (build.rs + capabilities), or Tauri rejects the call.
    #[test]
    fn every_command_is_allowed_for_the_panel() {
        let lib = include_str!("lib.rs");
        let build = include_str!("../build.rs");
        let caps = include_str!("../capabilities/panel.json");
        let start = lib.find("generate_handler![").expect("handler list") + "generate_handler![".len();
        let list = &lib[start..start + lib[start..].find(']').expect("end of list")];
        let mut checked = 0;
        for line in list.lines().map(str::trim) {
            let Some(name) = line.strip_suffix(',').and_then(|l| l.rsplit("::").next()) else { continue };
            if name.is_empty() || name.starts_with('#') {
                continue;
            }
            assert!(build.contains(&format!("\"{name}\"")), "{name} missing from build.rs COMMANDS");
            let allow = format!("\"allow-{}\"", name.replace('_', "-"));
            assert!(caps.contains(&allow), "{allow} missing from capabilities/panel.json");
            checked += 1;
        }
        assert!(checked > 20, "parsed the handler list ({checked})");
    }
}
