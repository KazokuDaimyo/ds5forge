//! Overlay window: a small transparent card displayed on top of games.
//!
//! It never takes focus (a windowed full-screen game would otherwise react to it) and the mouse
//! passes through it. It appears when the backend emits the "overlay" event, then hides itself.

use serde_json::Value;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tauri::{AppHandle, Emitter, LogicalPosition, Manager, WebviewUrl, WebviewWindowBuilder};

const LABEL: &str = "overlay";
const WIDTH: f64 = 440.0;
/// Tall enough for four controllers and a message; the window is transparent and click-through.
const HEIGHT: f64 = 560.0;
/// Distance between the top of the screen and the overlay.
const TOP_MARGIN: f64 = 24.0;
/// How long the overlay stays visible after the last trigger.
const VISIBLE_FOR: Duration = Duration::from_millis(3500);

/// Counts the displays: a delayed hide only takes effect if no other display happened since.
#[derive(Clone, Default)]
pub struct OverlayState(Arc<AtomicU64>);

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

    let _ = window.emit("overlay", payload);
    let _ = window.show();

    let state = app.state::<OverlayState>().inner().clone();
    let generation = state.0.fetch_add(1, Ordering::SeqCst) + 1;
    std::thread::spawn(move || {
        std::thread::sleep(VISIBLE_FOR);
        if state.0.load(Ordering::SeqCst) == generation {
            let _ = window.hide();
        }
    });
}
