//! Tray icon and the pop-up panel. The panel's webview only exists while it
//! is open: closing it destroys the window, so Butters idles without a
//! browser engine in memory.

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use tauri::image::Image;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, Rect, WebviewUrl, WebviewWindow, WebviewWindowBuilder, WindowEvent};

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
/// While Butters is closing things (and showing the result), the panel stays
/// open even though quitting an app hands focus to another app.
static HOLD: AtomicBool = AtomicBool::new(false);
static HOLD_RELEASED: Mutex<Option<Instant>> = Mutex::new(None);

/// Keep the panel open (true) while closing; on release, bring focus back to
/// the panel so a click elsewhere closes it as usual.
pub fn hold(app: &AppHandle, on: bool) {
    HOLD.store(on, Ordering::SeqCst);
    if !on {
        if let Ok(mut t) = HOLD_RELEASED.lock() {
            *t = Some(Instant::now());
        }
        if let Some(w) = app.get_webview_window(PANEL)
            && !LEAVING.load(Ordering::SeqCst) {
                let _ = w.set_focus();
            }
    }
}

fn held() -> bool {
    HOLD.load(Ordering::SeqCst)
        || HOLD_RELEASED.lock().map(|t| t.is_some_and(|t| t.elapsed() < Duration::from_millis(600))).unwrap_or(false)
}

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
        .tooltip(if cfg!(feature = "playground") { "Butters Playground" } else { "Butters" })
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
/// The global shortcut: open the panel (under the menu bar icon), or close it.
pub fn toggle_from_shortcut(app: &AppHandle) {
    if app.get_webview_window(PANEL).is_some() {
        close(app);
    } else {
        open(app, None);
    }
}

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
    HOLD.store(false, Ordering::SeqCst);
    position(app, &win, rect);
    if let Ok(mut t) = OPENED_AT.lock() {
        *t = Some(Instant::now());
    }
    state.panel_open.store(true, std::sync::atomic::Ordering::Relaxed);
    let handle = app.clone();
    win.on_window_event(move |event| match event {
        WindowEvent::Focused(false) => {
            let settling = OPENED_AT.lock().map(|t| t.is_some_and(|t| t.elapsed() < Duration::from_millis(300))).unwrap_or(false);
            if !settling && !held() {
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
///
/// All the maths happens in the target monitor's own pixels: with screens of
/// different scale (a Retina laptop next to a 1080p monitor), the panel
/// window's scale factor belongs to whichever screen it was created on, not
/// necessarily the one it is about to appear on.
fn position(app: &AppHandle, win: &WebviewWindow, rect: Option<Rect>) {
    let monitor = rect
        .and_then(|r| tray_monitor(app, win, r))
        .or_else(|| app.primary_monitor().ok().flatten());
    let Some(monitor) = monitor else { return };
    let scale = monitor.scale_factor();
    let area = monitor.work_area();
    let area = [area.position.x as f64, area.position.y as f64, area.size.width as f64, area.size.height as f64];
    let tray = rect.map(|r| {
        let (p, s) = (r.position.to_physical::<f64>(scale), r.size.to_physical::<f64>(scale));
        [p.x, p.y, s.width, s.height]
    });
    let (x, y) = place(tray, area, scale);
    // macOS places windows in points shared by every screen (monitor pixels
    // are points times that monitor's scale); elsewhere pixels are global.
    #[cfg(target_os = "macos")]
    let _ = win.set_position(tauri::LogicalPosition::new(x / scale, y / scale));
    #[cfg(not(target_os = "macos"))]
    let _ = win.set_position(tauri::PhysicalPosition::new(x.round() as i32, y.round() as i32));
}

/// The monitor the tray icon was clicked on.
fn tray_monitor(app: &AppHandle, win: &WebviewWindow, rect: Rect) -> Option<tauri::Monitor> {
    // macOS looks monitors up by points, but the tray reports pixels of its
    // own screen (whose scale we don't know yet). The pointer is on the icon
    // right after the click, and its position converts to points exactly.
    #[cfg(target_os = "macos")]
    {
        let _ = (win, rect);
        let primary = app.primary_monitor().ok().flatten()?;
        let at = app.cursor_position().ok()?;
        let s = primary.scale_factor();
        app.monitor_from_point(at.x / s, at.y / s).ok().flatten()
    }
    #[cfg(not(target_os = "macos"))]
    {
        let p = rect.position.to_physical::<f64>(win.scale_factor().unwrap_or(1.0));
        app.monitor_from_point(p.x, p.y).ok().flatten()
    }
}

/// The panel window's top-left corner, in the monitor's pixels. `tray` and
/// `area` (the monitor's work area) are `[x, y, width, height]`.
fn place(tray: Option<[f64; 4]>, area: [f64; 4], scale: f64) -> (f64, f64) {
    let [ax, ay, aw, ah] = area;
    let (w, h) = ((WIDTH + 2.0 * MARGIN.0) * scale, (HEIGHT + MARGIN.1 + MARGIN.2) * scale);
    let gap = 6.0 * scale;
    let (side, top, bottom) = (MARGIN.0 * scale, MARGIN.1 * scale, MARGIN.2 * scale);
    let (x, y) = match tray {
        Some([tx, ty, tw, th]) => {
            let x = tx + tw / 2.0 - w / 2.0;
            let top_half = ty < ay + ah / 2.0;
            let y = if top_half { ty + th + gap - top } else { ty - h - gap + bottom };
            (x, y)
        }
        None if cfg!(target_os = "macos") => (ax + aw - w - 2.0 * gap, ay + gap),
        None => (ax + aw - w - 2.0 * gap, ay + ah - h - 2.0 * gap),
    };
    // Keep the visible panel (not its transparent margin) on screen.
    (keep_inside(x, ax, aw, w, gap, side, side), keep_inside(y, ay, ah, h, gap, top, bottom))
}

/// One axis of "keep the visible panel in the work area" (`start`, `len`):
/// `size` is the window, `lead`/`trail` its transparent margins. A panel
/// too big for the area is pinned to the area's start.
fn keep_inside(pos: f64, start: f64, len: f64, size: f64, gap: f64, lead: f64, trail: f64) -> f64 {
    let lo = start + gap - lead;
    let hi = (start + len - size - gap + trail).max(start - lead);
    // Not f64::clamp: that panics (and aborts the app) whenever lo > hi,
    // which is exactly the too-big case.
    pos.max(lo).min(hi)
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
    let name = if cfg!(feature = "playground") { "Butters Playground" } else { "Butters" };
    let _ = tray.set_tooltip(Some(format!("{name}: {word}\n{}", crate::app::summary_line(stats))));
    let code = level as u8 + 1;
    if TRAY_LEVEL.swap(code, Ordering::Relaxed) != code {
        let bytes = if level == Level::Strained { TRAY_ICON_ALERT } else { TRAY_ICON };
        if let Ok(img) = Image::from_bytes(bytes) {
            let _ = tray.set_icon(Some(img));
            let _ = tray.set_icon_as_template(false);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_panel_too_tall_for_the_screen_is_pinned_instead_of_crashing() {
        // The v0.1.13 crash ("min > max ... min = 34.0, max = 22.0"): the
        // panel measured at Retina scale (1208 px tall) against a 1080p
        // monitor's work area (1050 px below a 30 px menu bar).
        let (gap, top, bottom) = (12.0, 8.0, 40.0);
        assert_eq!(keep_inside(40.0, 30.0, 1050.0, 1208.0, gap, top, bottom), 22.0);
        // Same when it is too wide.
        assert_eq!(keep_inside(500.0, 0.0, 300.0, 404.0, 6.0, 12.0, 12.0), -12.0);
    }

    #[test]
    fn panel_sits_under_the_tray_icon_and_stays_on_screen() {
        // 1080p monitor at scale 1, menu bar 30 px, icon near the right edge.
        let area = [0.0, 30.0, 1920.0, 1050.0];
        let (x, y) = place(Some([1880.0, 0.0, 30.0, 30.0]), area, 1.0);
        let (w, h) = (WIDTH + 2.0 * MARGIN.0, HEIGHT + MARGIN.1 + MARGIN.2);
        assert_eq!(y, 30.0 + 6.0 - MARGIN.1, "just below the menu bar");
        assert_eq!(x, 1920.0 - w - 6.0 + MARGIN.0, "pulled in from the right edge");
        assert!(y + h <= 30.0 + 1050.0);
    }

    #[test]
    fn panel_is_measured_in_the_target_monitors_pixels() {
        // Retina monitor to the right of a 1920-pt-wide one: its pixels are
        // points x 2, so it starts at x = 3840.
        let area = [3840.0, 74.0, 3456.0, 2160.0];
        let (x, y) = place(Some([5000.0, 0.0, 60.0, 74.0]), area, 2.0);
        assert_eq!(x, 5000.0 + 30.0 - (WIDTH + 2.0 * MARGIN.0));
        assert_eq!(y, 74.0 + 12.0 - 2.0 * MARGIN.1);
    }

    #[test]
    fn a_screen_smaller_than_the_panel_does_not_crash() {
        let (x, y) = place(Some([300.0, 0.0, 20.0, 20.0]), [0.0, 20.0, 320.0, 400.0], 1.0);
        assert_eq!((x, y), (-MARGIN.0, 20.0 - MARGIN.1));
        let _ = place(None, [0.0, 0.0, 1.0, 1.0], 3.0);
    }
}
