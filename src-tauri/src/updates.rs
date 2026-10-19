//! Updates from GitHub Releases. Butters checks quietly (shortly after start,
//! then every 6 hours) and tells the user; it only installs when asked.
//! The updater plugin refuses any download whose signature doesn't match the
//! public key built into the app (tauri.conf.json > plugins.updater.pubkey).

use std::sync::Mutex;
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Manager};
use tauri_plugin_updater::{Update, UpdaterExt};

const FIRST_CHECK: Duration = Duration::from_secs(20);
const EVERY: Duration = Duration::from_secs(6 * 60 * 60);

#[derive(Default)]
pub struct UpdateState {
    pending: Mutex<Option<Update>>,
    status: Mutex<Status>,
    notified: Mutex<Option<String>>,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub current: String,
    /// Newer version ready to install, if any.
    pub available: Option<String>,
    pub checking: bool,
    pub installing: bool,
    pub error: Option<String>,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

fn short(e: impl std::fmt::Display) -> String {
    let s = e.to_string();
    let first = s.lines().next().unwrap_or("").trim();
    first.chars().take(80).collect()
}

async fn check(app: &AppHandle) -> Status {
    let state = app.state::<UpdateState>();
    let current = app.package_info().version.to_string();
    lock(&state.status).checking = true;
    let result = match app.updater() {
        Ok(u) => u.check().await.map_err(short),
        Err(e) => Err(short(e)),
    };
    let mut status = Status { current, ..Default::default() };
    match result {
        Ok(Some(update)) => {
            status.available = Some(update.version.clone());
            *lock(&state.pending) = Some(update);
        }
        Ok(None) => *lock(&state.pending) = None,
        Err(e) => status.error = Some(e),
    }
    *lock(&state.status) = status.clone();
    status
}

pub fn status(app: &AppHandle) -> Status {
    let mut s = lock(&app.state::<UpdateState>().status).clone();
    if s.current.is_empty() {
        s.current = app.package_info().version.to_string();
    }
    s
}

#[tauri::command]
pub fn update_status(app: AppHandle) -> Status {
    status(&app)
}

#[tauri::command]
pub async fn check_update(app: AppHandle) -> Status {
    check(&app).await
}

#[tauri::command]
pub async fn install_update(app: AppHandle) -> Result<(), String> {
    let state = app.state::<UpdateState>();
    let update = lock(&state.pending).take().ok_or("No update ready. Check again.")?;
    lock(&state.status).installing = true;
    match update.download_and_install(|_, _| {}, || {}).await {
        Ok(()) => app.restart(),
        Err(e) => {
            let mut s = lock(&state.status);
            s.installing = false;
            s.error = Some(short(e));
            Err("Update failed. Try again later.".into())
        }
    }
}

/// Background checks. Notifies once per new version; never installs on its own.
pub fn spawn_checks(app: AppHandle) {
    std::thread::Builder::new()
        .name("butters-updates".into())
        .spawn(move || {
            std::thread::sleep(FIRST_CHECK);
            loop {
                let status = tauri::async_runtime::block_on(check(&app));
                if let Some(v) = status.available {
                    let state = app.state::<UpdateState>();
                    let mut notified = lock(&state.notified);
                    if notified.as_deref() != Some(v.as_str()) {
                        *notified = Some(v.clone());
                        crate::permissions::notify(&app, &format!("Update {v} ready"));
                    }
                }
                std::thread::sleep(EVERY);
            }
        })
        .expect("spawn update thread");
}
