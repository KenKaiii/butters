//! Tray icon and the pop-up panel. The panel's webview only exists while it
//! is open: closing it destroys the window, so Butters idles without a
//! browser engine in memory.

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use tauri::image::Image;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, PhysicalPosition, Rect, WebviewUrl, WebviewWindow, WebviewWindowBuilder, WindowEvent};

use crate::alerts::Level;
use crate::app::AppState;
use crate::sampler::Stats;

const PANEL: &str = "panel";
const TRAY: &str = "butters";
const WIDTH: f64 = 380.0;
const HEIGHT: f64 = 580.0;
/// macOS draws the panel inside a transparent window with room for its own
/// soft shadow (matches the CSS margins of `html.floating .panel`).
#[cfg(target_os = "macos")]
const MARGIN: (f64, f64, f64) = (12.0, 4.0, 20.0); // sides, top, bottom
#[cfg(not(target_os = "macos"))]
const MARGIN: (f64, f64, f64) = (0.0, 0.0, 0.0);
/// How long the panel gets to melt away before the window is destroyed.
const LEAVE: Duration = Duration::from_millis(270);

/// When the panel last closed because it lost focus. Clicking the tray icon
/// while the panel is open first blurs it; that click must not reopen it.
static CLOSED_BY_BLUR: Mutex<Option<Instant>> = Mutex::new(None);
static OPENED_AT: Mutex<Option<Instant>> = Mutex::new(None);
static TRAY_LEVEL: AtomicU8 = AtomicU8::new(0);
/// Bumped for every new panel, so a late close can't hit a newer panel.
static GENERATION: AtomicU64 = AtomicU64::new(0);
static LEAVING: AtomicBool = AtomicBool::new(false);

// Colour icons on every platform (not macOS template images), so the menu
// bar shows an actual pat of butter. Drawn by scripts/make_icons.py.
const TRAY_ICON: &[u8] = include_bytes!("../icons/tray.png");
const TRAY_ICON_ALERT: &[u8] = include_bytes!("../icons/tray-alert.png");

pub fn setup_tray(app: &AppHandle) -> tauri::Result<()> {
    let open_item = MenuItem::with_id(app, "open", "Open Butters", true, None::<&str>)?;
    let quit_item = MenuItem::with_id(app, "quit", "Quit Butters", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open_item, &quit_item])?;
    TrayIconBuilder::with_id(TRAY)
        .icon(Image::from_bytes(TRAY_ICON)?)
        .icon_as_template(false)
        .tooltip("Butters")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "open" => open(app, None),
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, rect, .. } = event {
                toggle(tray.app_handle(), rect);
            }
        })
        .build(app)?;
    Ok(())
}

fn toggle(app: &AppHandle, rect: Rect) {
    if app.get_webview_window(PANEL).is_some() {
        close(app);
        return;
    }
    let just_closed = CLOSED_BY_BLUR
        .lock()
        .map(|g| g.is_some_and(|t| t.elapsed() < Duration::from_millis(400)))
        .unwrap_or(false);
    if !just_closed {
        open(app, Some(rect));
    }
}

/// Lets the panel play its melt-away, then destroys the window (and with it
/// the web view's memory).
pub fn close(app: &AppHandle) {
    let Some(w) = app.get_webview_window(PANEL) else { return };
    if LEAVING.swap(true, Ordering::SeqCst) {
        return; // already on its way out
    }
    // Static script, no user data: just triggers the CSS exit state.
    let _ = w.eval("window.__buttersLeave && window.__buttersLeave()");
    crate::sound::play(app, "close");
    let generation = GENERATION.load(Ordering::SeqCst);
    let app = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(LEAVE);
        if GENERATION.load(Ordering::SeqCst) == generation
            && let Some(w) = app.get_webview_window(PANEL) {
                let _ = w.destroy();
            }
    });
}

pub fn open(app: &AppHandle, rect: Option<Rect>) {
    if let Some(w) = app.get_webview_window(PANEL) {
        if !LEAVING.load(Ordering::SeqCst) {
            let _ = w.set_focus();
        }
        return;
    }
    let state = app.state::<AppState>();
    // Note the app in front *before* the panel takes focus: that's the one to protect.
    state.front.lock().unwrap_or_else(|e| e.into_inner()).poll();

    let builder = WebviewWindowBuilder::new(app, PANEL, WebviewUrl::App("index.html".into()))
        .title("Butters")
        .inner_size(WIDTH + 2.0 * MARGIN.0, HEIGHT + MARGIN.1 + MARGIN.2)
        .resizable(false)
        .maximizable(false)
        .minimizable(false)
        .decorations(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .visible(false)
        .focused(true)
        // The panel is always cream; keep native bits (scrollbars) light too.
        .theme(Some(tauri::Theme::Light));
    #[cfg(target_os = "macos")]
    let builder = builder.transparent(true).shadow(false);
    #[cfg(not(target_os = "macos"))]
    let builder = builder.shadow(true);
    let Ok(win) = builder.build() else { return };
    GENERATION.fetch_add(1, Ordering::SeqCst);
    LEAVING.store(false, Ordering::SeqCst);
    position(app, &win, rect);
    if let Ok(mut t) = OPENED_AT.lock() {
        *t = Some(Instant::now());
    }
    state.panel_open.store(true, std::sync::atomic::Ordering::Relaxed);
    let handle = app.clone();
    win.on_window_event(move |event| match event {
        WindowEvent::Focused(false) => {
            let settling = OPENED_AT.lock().map(|t| t.is_some_and(|t| t.elapsed() < Duration::from_millis(300))).unwrap_or(false);
            if !settling {
                if let Ok(mut t) = CLOSED_BY_BLUR.lock() {
                    *t = Some(Instant::now());
                }
                close(&handle);
            }
        }
        WindowEvent::Destroyed => {
            handle.state::<AppState>().panel_open.store(false, std::sync::atomic::Ordering::Relaxed);
        }
        _ => {}
    });
    let _ = win.show();
    let _ = win.set_focus();
}

/// Places the panel next to the tray icon (below it in the macOS menu bar,
/// above it on a bottom taskbar), kept inside the screen's work area.
fn position(app: &AppHandle, win: &WebviewWindow, rect: Option<Rect>) {
    let scale = win.scale_factor().unwrap_or(1.0);
    let Ok(size) = win.outer_size() else { return };
    let (w, h) = (size.width as f64, size.height as f64);
    let gap = 6.0 * scale;
    let (side, top, bottom) = (MARGIN.0 * scale, MARGIN.1 * scale, MARGIN.2 * scale);

    let tray = rect.map(|r| (r.position.to_physical::<f64>(scale), r.size.to_physical::<f64>(scale)));
    let monitor = match tray {
        Some((p, _)) => app.monitor_from_point(p.x, p.y).ok().flatten(),
        None => None,
    }
    .or_else(|| app.primary_monitor().ok().flatten());
    let Some(monitor) = monitor else { return };
    let area = monitor.work_area();
    let (ax, ay) = (area.position.x as f64, area.position.y as f64);
    let (aw, ah) = (area.size.width as f64, area.size.height as f64);

    let (mut x, mut y) = match tray {
        Some((p, s)) => {
            let x = p.x + s.width / 2.0 - w / 2.0;
            let top_half = p.y < ay + ah / 2.0;
            let y = if top_half { p.y + s.height + gap - top } else { p.y - h - gap + bottom };
            (x, y)
        }
        None if cfg!(target_os = "macos") => (ax + aw - w - 2.0 * gap, ay + gap),
        None => (ax + aw - w - 2.0 * gap, ay + ah - h - 2.0 * gap),
    };
    // Keep the visible panel (not its transparent margin) on screen.
    x = x.clamp(ax + gap - side, (ax + aw - w - gap + side).max(ax - side));
    y = y.clamp(ay + gap - top, (ay + ah - h - gap + bottom).max(ay - top));
    let _ = win.set_position(PhysicalPosition::new(x.round() as i32, y.round() as i32));
}

/// Tray tooltip shows the current state; the icon gains a dot when the
/// machine is under strain.
pub fn set_tray_level(app: &AppHandle, level: Level, stats: &Stats) {
    let Some(tray) = app.tray_by_id(TRAY) else { return };
    let word = match level {
        Level::Smooth => "Running smooth",
        Level::Busy => "Getting busy",
        Level::Strained => "Under strain",
    };
    let _ = tray.set_tooltip(Some(format!("Butters: {word}\n{}", crate::app::summary_line(stats))));
    let code = level as u8 + 1;
    if TRAY_LEVEL.swap(code, Ordering::Relaxed) != code {
        let bytes = if level == Level::Strained { TRAY_ICON_ALERT } else { TRAY_ICON };
        if let Ok(img) = Image::from_bytes(bytes) {
            let _ = tray.set_icon(Some(img));
            let _ = tray.set_icon_as_template(false);
        }
    }
}
