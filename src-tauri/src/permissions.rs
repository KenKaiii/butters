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

/// Shows a Butters notification (title "Butters", one short line).
/// Clicking it opens the Butters panel.
pub fn notify(app: &tauri::AppHandle, body: &str) {
    // Its own thread: the first one may wait on the system's permission
    // prompt, which must not stall the background checks.
    let app = app.clone();
    let body = body.to_string();
    let _ = std::thread::Builder::new().name("butters-notify".into()).spawn(move || imp::notify(&app, &body));
}

/// Call once at startup: lets a click on a Butters notification open the panel.
pub fn setup(app: &tauri::AppHandle) {
    imp::setup(app);
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

    use block2::{DynBlock, RcBlock};
    use objc2::rc::Retained;
    use objc2::runtime::{Bool, NSObject, NSObjectProtocol, ProtocolObject};
    use objc2::{AnyThread, DefinedClass, define_class, msg_send};
    use objc2_foundation::{NSError, NSString, NSUUID};
    use objc2_user_notifications::{
        UNAuthorizationOptions, UNAuthorizationStatus, UNMutableNotificationContent, UNNotification,
        UNNotificationPresentationOptions, UNNotificationRequest, UNNotificationResponse, UNNotificationSettings,
        UNNotificationSound, UNUserNotificationCenter, UNUserNotificationCenterDelegate,
    };
    use tauri::AppHandle;

    use super::Notify;

    pub struct DelegateIvars {
        app: AppHandle,
    }

    define_class!(
        // SAFETY: NSObject has no subclassing requirements; no Drop impl.
        #[unsafe(super(NSObject))]
        #[name = "ButtersNotificationDelegate"]
        #[ivars = DelegateIvars]
        struct NotificationDelegate;

        unsafe impl NSObjectProtocol for NotificationDelegate {}

        unsafe impl UNUserNotificationCenterDelegate for NotificationDelegate {
            // A click on a Butters notification opens the panel.
            #[unsafe(method(userNotificationCenter:didReceiveNotificationResponse:withCompletionHandler:))]
            fn did_receive(
                &self,
                _center: &UNUserNotificationCenter,
                _response: &UNNotificationResponse,
                completion: &DynBlock<dyn Fn()>,
            ) {
                let app = self.ivars().app.clone();
                let open = app.clone();
                let _ = app.run_on_main_thread(move || crate::panel::open(&open, None));
                completion.call(());
            }

            // Still show the banner if Butters happens to be frontmost.
            #[unsafe(method(userNotificationCenter:willPresentNotification:withCompletionHandler:))]
            fn will_present(
                &self,
                _center: &UNUserNotificationCenter,
                _notification: &UNNotification,
                completion: &DynBlock<dyn Fn(UNNotificationPresentationOptions)>,
            ) {
                completion.call((UNNotificationPresentationOptions::Banner | UNNotificationPresentationOptions::List,));
            }
        }
    );

    pub fn setup(app: &AppHandle) {
        if !bundled() {
            return;
        }
        let this = NotificationDelegate::alloc().set_ivars(DelegateIvars { app: app.clone() });
        // SAFETY: plain NSObject init on a freshly allocated instance.
        let delegate: Retained<NotificationDelegate> = unsafe { msg_send![super(this), init] };
        UNUserNotificationCenter::currentNotificationCenter().setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
        // The center only holds the delegate weakly; keep it for the app's lifetime.
        std::mem::forget(delegate);
    }

    /// Posts through UNUserNotificationCenter (the current API). If Butters
    /// hasn't asked yet, the system prompt appears first, once.
    pub fn notify(_app: &AppHandle, body: &str) {
        if !bundled() {
            return;
        }
        let allowed = match state() {
            Notify::On => true,
            Notify::Ask => request() == Notify::On,
            Notify::Off => false,
        };
        if !allowed {
            return;
        }
        let content = UNMutableNotificationContent::new();
        content.setTitle(&NSString::from_str("Butters"));
        content.setBody(&NSString::from_str(body));
        content.setSound(Some(&UNNotificationSound::defaultSound()));
        content.setThreadIdentifier(&NSString::from_str("butters"));
        let id = NSUUID::new().UUIDString();
        let request = UNNotificationRequest::requestWithIdentifier_content_trigger(&id, &content, None);
        let done = RcBlock::new(|err: *mut NSError| {
            if !err.is_null() {
                // SAFETY: non-null NSError passed by the system for this callback.
                eprintln!("notification failed: {}", unsafe { &*err }.localizedDescription());
            }
        });
        UNUserNotificationCenter::currentNotificationCenter().addNotificationRequest_withCompletionHandler(&request, Some(&done));
    }

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

    pub fn setup(_app: &tauri::AppHandle) {}

    pub fn notify(app: &tauri::AppHandle, body: &str) {
        use tauri_plugin_notification::NotificationExt;
        let _ = app.notification().builder().title("Butters").body(body).show();
    }

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
    pub fn setup(_app: &tauri::AppHandle) {}
    pub fn notify(app: &tauri::AppHandle, body: &str) {
        use tauri_plugin_notification::NotificationExt;
        let _ = app.notification().builder().title("Butters").body(body).show();
    }
    pub fn state() -> Notify {
        Notify::On
    }
    pub fn request() -> Notify {
        Notify::On
    }
    pub fn open_settings() {}
}
