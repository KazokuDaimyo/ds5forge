//! Overlay window: a small transparent card displayed on top of games.
//!
//! It never takes focus (a windowed full-screen game would otherwise react to it) and the mouse
//! passes through it. It appears when the backend emits the "overlay" event, then hides itself.

use serde_json::Value;
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tauri::{AppHandle, Emitter, LogicalPosition, Manager, WebviewUrl, WebviewWindowBuilder};

const LABEL: &str = "overlay";
/// The card is 420 wide: the rest is the transparent margin that holds its shadow.
const WIDTH: f64 = 480.0;
/// Tall enough for four controllers and a message; the window is transparent and click-through.
const HEIGHT: f64 = 600.0;
/// Distance between the top of the screen and the overlay window (its transparent margin is 30, so the
/// card itself sits 34 from the top).
const TOP_MARGIN: f64 = 4.0;
/// How long the overlay stays visible after the last trigger.
const VISIBLE_FOR: Duration = Duration::from_millis(3500);

/// What the overlay remembers between two displays.
#[derive(Clone, Default)]
pub struct OverlayState {
    /// Counts the displays: a delayed hide only takes effect if no other display happened since.
    generation: Arc<AtomicU64>,
    /// The overlay window holds the focus (the controller menu is open and asked for it).
    focused: Arc<AtomicBool>,
    /// The window that had the focus before, to give it back (0: none).
    previous_window: Arc<AtomicIsize>,
}

/// Creates the (hidden) window. To be called at startup.
pub fn create(app: &AppHandle) -> tauri::Result<()> {
    let window = WebviewWindowBuilder::new(app, LABEL, WebviewUrl::App("overlay.html".into()))
        .title("DS5Forge Overlay")
        .inner_size(WIDTH, HEIGHT)
        .decorations(false)
        .transparent(true)
        .shadow(false)
        .resizable(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .focusable(false)
        .focused(false)
        .visible(false)
        .build()?;
    window.set_ignore_cursor_events(true)?;
    Ok(())
}

/// Shows the overlay with `payload`, centred at the top of the primary screen.
pub fn show(app: &AppHandle, payload: Value) {
    let Some(window) = app.get_webview_window(LABEL) else {
        return;
    };

    if let Ok(Some(monitor)) = app.primary_monitor() {
        let scale = monitor.scale_factor();
        let origin = monitor.position().to_logical::<f64>(scale);
        let width = monitor.size().to_logical::<f64>(scale).width;
        let _ = window.set_position(LogicalPosition::new(
            origin.x + (width - WIDTH) / 2.0,
            origin.y + TOP_MARGIN,
        ));
    }

    // The controller menu stays until it is closed: only a message hides itself.
    let menu_open = payload.get("menu").is_some_and(|menu| !menu.is_null());
    let wants_focus = menu_open && payload.get("take_focus").and_then(Value::as_bool).unwrap_or(false);
    let _ = window.emit("overlay", payload);
    let _ = window.show();

    let state = app.state::<OverlayState>().inner().clone();
    // A new display always cancels the hide planned by the previous one.
    let generation = state.generation.fetch_add(1, Ordering::SeqCst) + 1;
    if menu_open {
        if wants_focus {
            take_focus(&window, &state);
        }
        return;
    }
    std::thread::spawn(move || {
        std::thread::sleep(VISIBLE_FOR);
        if state.generation.load(Ordering::SeqCst) == generation {
            let _ = window.hide();
        }
    });
}

/// Hides the overlay at once and cancels any hide that was planned.
pub fn hide(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(LABEL) {
        let state = app.state::<OverlayState>().inner().clone();
        state.generation.fetch_add(1, Ordering::SeqCst);
        let _ = window.hide();
        release_focus(&window, &state);
    }
}

/// Gives the focus to the overlay window while the controller menu is open. The game, no longer in
/// the foreground, then stops reacting to the controller (most games do not read it in the
/// background). The window that had the focus is remembered, to give it back afterwards.
fn take_focus(window: &tauri::WebviewWindow, state: &OverlayState) {
    if state.focused.swap(true, Ordering::SeqCst) {
        return; // already holding it: the menu is only being redrawn
    }
    state.previous_window.store(foreground_window(), Ordering::SeqCst);
    let _ = window.set_focusable(true);
    let _ = window.set_focus();
}

/// Makes the overlay window unable to take the focus again, and gives the focus back.
fn release_focus(window: &tauri::WebviewWindow, state: &OverlayState) {
    if !state.focused.swap(false, Ordering::SeqCst) {
        return;
    }
    let _ = window.set_focusable(false);
    restore_foreground(state.previous_window.swap(0, Ordering::SeqCst));
}

/// The window that has the focus, as a handle (0 if there is none).
#[cfg(windows)]
fn foreground_window() -> isize {
    // SAFETY: a plain query.
    unsafe { windows_sys::Win32::UI::WindowsAndMessaging::GetForegroundWindow() as isize }
}

/// Gives the focus back to `window` if it still exists.
#[cfg(windows)]
fn restore_foreground(window: isize) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{IsWindow, SetForegroundWindow};
    if window != 0 {
        // SAFETY: the handle is only passed back to Windows, which checks it.
        unsafe {
            if IsWindow(window as _) != 0 {
                SetForegroundWindow(window as _);
            }
        }
    }
}

#[cfg(not(windows))]
fn foreground_window() -> isize {
    0
}

#[cfg(not(windows))]
fn restore_foreground(_window: isize) {}
