pub mod actions;
pub mod alerts;
pub mod app;
pub mod docker_activity;
pub mod engine;
pub mod history;
pub mod panel;
pub mod permissions;
pub mod platform;
pub mod sampler;
pub mod services;
pub mod settings;
pub mod trends;
pub mod sound;
pub mod updates;

use tauri::Manager;

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
            let config_dir = app.path().app_config_dir()?;
            app.manage(app::AppState::new(settings::Settings::path(&config_dir)));
            panel::setup_tray(app.handle())?;
            permissions::setup(app.handle());
            app::apply_shortcut(app.handle(), "");
            app::spawn_background(app.handle().clone());
            updates::spawn_checks(app.handle().clone());
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
        ])
        .build(tauri::generate_context!())
        .expect("error while starting Butters")
        .run(|_app, event| {
            // Closing the panel must not quit the app; only "Quit Butters" does.
            if let tauri::RunEvent::ExitRequested { code: None, api, .. } = event {
                api.prevent_exit();
            }
        });
}
