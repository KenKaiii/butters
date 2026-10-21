pub mod actions;
pub mod alerts;
pub mod app;
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
            // Menu bar app: no Dock icon, no app switcher entry.
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
