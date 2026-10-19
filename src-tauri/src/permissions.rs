//! The one permission Butters needs: notifications (for alerts and "update
//! ready"). Everything else (reading your own processes, which app is in
//! front, closing your own apps) needs no permission on macOS or Windows.
//!
//! The panel shows a single "Turn on" row only while notifications are
//! undecided or off, so the user can fix it in one click.

use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Notify {
    /// Never asked: one click shows the system prompt.
    Ask,
    /// Turned off in System Settings: one click opens the right page.
    Off,
    On,
}

#[tauri::command]
pub async fn notify_permission() -> Notify {
    tauri::async_runtime::spawn_blocking(imp::state).await.unwrap_or(Notify::On)
}

/// Asks the system (if it hasn't been asked yet), otherwise opens
/// Notification settings. Returns the state afterwards.
#[tauri::command]
pub async fn fix_notify_permission() -> Notify {
    tauri::async_runtime::spawn_blocking(|| match imp::state() {
        Notify::Ask => imp::request(),
        Notify::Off => {
            imp::open_settings();
            Notify::Off
        }
        Notify::On => Notify::On,
    })
    .await
    .unwrap_or(Notify::On)
}

#[cfg(target_os = "macos")]
mod imp {
    use std::sync::mpsc;
    use std::time::Duration;

    use block2::RcBlock;
    use objc2::runtime::Bool;
    use objc2_foundation::NSError;
    use objc2_user_notifications::{
        UNAuthorizationOptions, UNAuthorizationStatus, UNNotificationSettings, UNUserNotificationCenter,
    };

    use super::Notify;

    /// UNUserNotificationCenter only works inside a real .app bundle;
    /// calling it from `cargo run`/tests would crash the process.
    fn bundled() -> bool {
        std::env::current_exe().map(|p| p.to_string_lossy().contains(".app/Contents/MacOS/")).unwrap_or(false)
    }

    pub fn state() -> Notify {
        if !bundled() {
            return Notify::On;
        }
        let (tx, rx) = mpsc::channel();
        let block = RcBlock::new(move |settings: std::ptr::NonNull<UNNotificationSettings>| {
            // SAFETY: the system passes a valid settings object for the duration of the callback.
            let status = unsafe { settings.as_ref() }.authorizationStatus();
            let _ = tx.send(status);
        });
        UNUserNotificationCenter::currentNotificationCenter().getNotificationSettingsWithCompletionHandler(&block);
        match rx.recv_timeout(Duration::from_secs(3)) {
            Ok(UNAuthorizationStatus::NotDetermined) => Notify::Ask,
            Ok(UNAuthorizationStatus::Denied) => Notify::Off,
            Ok(_) => Notify::On,
            Err(_) => Notify::On, // don't nag if the system didn't answer
        }
    }

    pub fn request() -> Notify {
        if !bundled() {
            return Notify::On;
        }
        let (tx, rx) = mpsc::channel();
        let block = RcBlock::new(move |granted: Bool, _err: *mut NSError| {
            let _ = tx.send(granted.as_bool());
        });
        UNUserNotificationCenter::currentNotificationCenter().requestAuthorizationWithOptions_completionHandler(
            UNAuthorizationOptions::Alert | UNAuthorizationOptions::Sound,
            &block,
        );
        // The user may take a while to click Allow.
        match rx.recv_timeout(Duration::from_secs(120)) {
            Ok(true) => Notify::On,
            Ok(false) => Notify::Off,
            Err(_) => state(),
        }
    }

    pub fn open_settings() {
        // Fixed URL to the Notifications pane; no user input involved.
        let _ = std::process::Command::new("/usr/bin/open")
            .arg("x-apple.systempreferences:com.apple.Notifications-Settings.extension")
            .status();
    }
}

#[cfg(windows)]
mod imp {
    use super::Notify;

    pub fn state() -> Notify {
        // Windows shows app notifications by default; nothing to ask.
        Notify::On
    }
    pub fn request() -> Notify {
        Notify::On
    }
    pub fn open_settings() {
        let _ = std::process::Command::new("explorer.exe").arg("ms-settings:notifications").status();
    }
}

#[cfg(not(any(target_os = "macos", windows)))]
mod imp {
    use super::Notify;
    pub fn state() -> Notify {
        Notify::On
    }
    pub fn request() -> Notify {
        Notify::On
    }
    pub fn open_settings() {}
}
