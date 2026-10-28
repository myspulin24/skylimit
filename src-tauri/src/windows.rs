//! The two windows.
//!
//! * `widget`: transparent, frameless, always on top, created at startup and
//!   only ever hidden (never closed). A tool window, so it stays out of the
//!   taskbar and Alt+Tab. Click-through is applied here.
//! * `app`: a normal window created on demand and destroyed on close, so an
//!   idle app holds only the widget's webview in memory.
//!
//! Both remember their placement (`window_state`).

use tauri::webview::Color;
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, WebviewUrl, WebviewWindow, WebviewWindowBuilder, WindowEvent};

use crate::window_state::{self, WindowTracker};
use crate::{shell, tray};

pub const WIDGET: &str = "widget";
pub const APP: &str = "app";

const WIDGET_SIZE: (f64, f64) = (360.0, 480.0);
/// Gap between the widget and the work-area edge on first run (logical px).
const WIDGET_MARGIN: f64 = 16.0;
const APP_SIZE: (f64, f64) = (1100.0, 760.0);
const APP_MIN_SIZE: (f64, f64) = (720.0, 520.0);
/// The app page's body is transparent; the window paints the surface color.
const APP_BACKGROUND: Color = Color(0x11, 0x14, 0x19, 0xff);

pub struct Placements {
    pub widget: WindowTracker,
    pub app: WindowTracker,
}

impl Placements {
    pub fn save_all(&self) {
        self.widget.save();
        self.app.save();
    }
}

// ----------------------------------------------------------------- widget ---

pub fn create_widget(app: &AppHandle, click_through: bool) -> tauri::Result<()> {
    let window = WebviewWindowBuilder::new(app, WIDGET, WebviewUrl::App("index.html?window=widget".into()))
        .title("Skylimit")
        .inner_size(WIDGET_SIZE.0, WIDGET_SIZE.1)
        .resizable(false)
        .maximizable(false)
        .minimizable(false)
        .decorations(false)
        .transparent(true)
        .shadow(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .visible(false)
        .build()?;

    // Tool-window style first: Get/SetWindowPlacement use screen coordinates
    // for tool windows, so saving and restoring must see the same style.
    window_state::make_tool_window(&window);
    place_widget(app, &window);

    let handle = window.clone();
    window.on_window_event(move |event| {
        let app = handle.app_handle();
        match event {
            // Alt+F4 or similar: the widget hides, it never closes.
            WindowEvent::CloseRequested { api, .. } => {
                api.prevent_close();
                hide_widget(app);
            }
            WindowEvent::Moved(_) => app.state::<Placements>().widget.capture(&handle),
            // Dragging ends with a click elsewhere sooner or later.
            WindowEvent::Focused(false) => app.state::<Placements>().widget.save(),
            _ => {}
        }
    });

    show_widget(app);
    if click_through {
        apply_click_through(app, true);
    }
    Ok(())
}

/// Saved position (keeping the widget's own size), else bottom-right of the
/// primary monitor's work area.
fn place_widget(app: &AppHandle, window: &WebviewWindow) {
    let tracker = &app.state::<Placements>().widget;
    let restored = match (tracker.stored(), window.outer_size()) {
        (Some(saved), Ok(size)) => {
            let placement = saved.with_size(size.width as i32, size.height as i32);
            window_state::apply(window, &placement, true)
        }
        _ => false,
    };
    if !restored {
        place_bottom_right(window);
    }
    tracker.capture(window);
}

fn place_bottom_right(window: &WebviewWindow) {
    let monitor = match window.primary_monitor() {
        Ok(Some(m)) => m,
        _ => return,
    };
    let Ok(size) = window.outer_size() else {
        return;
    };
    let area = monitor.work_area();
    let margin = (WIDGET_MARGIN * monitor.scale_factor()).round() as i32;
    let x = area.position.x + area.size.width as i32 - size.width as i32 - margin;
    let y = area.position.y + area.size.height as i32 - size.height as i32 - margin;
    let _ = window.set_position(PhysicalPosition::new(x, y));
}

pub fn show_widget(app: &AppHandle) {
    let Some(window) = app.get_webview_window(WIDGET) else {
        return;
    };
    let _ = window.show();
    keep_off_alt_tab(app);
}

pub fn hide_widget(app: &AppHandle) {
    let Some(window) = app.get_webview_window(WIDGET) else {
        return;
    };
    let tracker = &app.state::<Placements>().widget;
    tracker.capture(&window);
    tracker.save();
    let _ = window.hide();
    keep_off_alt_tab(app);
}

/// Tray click and menu item. While paused, showing the widget ends the pause.
pub fn toggle_widget(app: &AppHandle) {
    if shell::is_paused(app) {
        set_paused(app, false);
        return;
    }
    let Some(window) = app.get_webview_window(WIDGET) else {
        return;
    };
    if window.is_visible().unwrap_or(false) {
        hide_widget(app);
    } else {
        show_widget(app);
    }
}

/// Second launch of the app: bring the widget back and focus it.
pub fn reveal_widget(app: &AppHandle) {
    if shell::is_paused(app) {
        set_paused(app, false);
    } else {
        show_widget(app);
    }
    if let Some(window) = app.get_webview_window(WIDGET) {
        let _ = window.set_focus();
    }
}

/// "Pauza" in the tray: hides the widget and mutes the tray icon to mono.
/// The core keeps collecting data; nothing is stopped.
pub fn set_paused(app: &AppHandle, on: bool) {
    shell::set_paused_flag(app, on);
    tray::set_pause_checked(app, on);
    if on {
        hide_widget(app);
    } else {
        show_widget(app);
    }
    tray::refresh(app);
}

/// Click-through on the widget. Never while DevTools are open (debug builds),
/// otherwise the inspector could not be used on the widget.
pub fn apply_click_through(app: &AppHandle, on: bool) {
    let Some(window) = app.get_webview_window(WIDGET) else {
        return;
    };
    let on = on && !devtools_open(&window);
    if let Err(e) = window.set_ignore_cursor_events(on) {
        eprintln!("cannot set click-through: {e}");
    }
    keep_off_alt_tab(app);
}

#[cfg(debug_assertions)]
fn devtools_open(window: &WebviewWindow) -> bool {
    window.is_devtools_open()
}

#[cfg(not(debug_assertions))]
fn devtools_open(_window: &WebviewWindow) -> bool {
    false
}

/// tao rewrites the extended style on every flag change (show, hide,
/// click-through, ...) and drops `WS_EX_TOOLWINDOW`, which would put the
/// widget into Alt+Tab. Re-apply it on the main thread, queued after the
/// change that caused it, and remove a taskbar button the shell may have
/// added while the style was missing.
fn keep_off_alt_tab(app: &AppHandle) {
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        if let Some(window) = handle.get_webview_window(WIDGET) {
            window_state::make_tool_window(&window);
            let _ = window.set_skip_taskbar(true);
        }
    });
}

// -------------------------------------------------------------------- app ---

/// Focus the app window, creating it if needed.
pub fn open_app(app: &AppHandle) -> tauri::Result<()> {
    open_app_with(app, false)
}

/// Open the app window with the feedback dialog showing (tray menu).
pub fn open_feedback(app: &AppHandle) -> tauri::Result<()> {
    open_app_with(app, true)
}

/// A running window gets the `open-feedback` event; a new one reads
/// `feedback=1` from its URL, because it cannot listen before it loads.
fn open_app_with(app: &AppHandle, feedback: bool) -> tauri::Result<()> {
    if let Some(window) = app.get_webview_window(APP) {
        focus(&window);
        if feedback {
            let _ = window.emit_to(APP, "open-feedback", ());
        }
        return Ok(());
    }
    let url = if feedback { "index.html?window=app&feedback=1" } else { "index.html?window=app" };
    let built = WebviewWindowBuilder::new(app, APP, WebviewUrl::App(url.into()))
        .title("Skylimit")
        .inner_size(APP_SIZE.0, APP_SIZE.1)
        .min_inner_size(APP_MIN_SIZE.0, APP_MIN_SIZE.1)
        .center()
        .background_color(APP_BACKGROUND)
        .visible(false)
        .build();
    let window = match built {
        Ok(window) => window,
        Err(e) => {
            // A second click may have created it in the meantime.
            if let Some(window) = app.get_webview_window(APP) {
                focus(&window);
                return Ok(());
            }
            return Err(e);
        }
    };

    let tracker = &app.state::<Placements>().app;
    if let Some(saved) = tracker.stored() {
        if !window_state::apply(&window, &saved, false) {
            eprintln!("cannot restore the app window placement");
        }
    }
    focus(&window);
    tracker.capture(&window);

    let handle = window.clone();
    window.on_window_event(move |event| {
        let tracker = &handle.app_handle().state::<Placements>().app;
        match event {
            WindowEvent::Moved(_) | WindowEvent::Resized(_) => tracker.capture(&handle),
            // Not prevented: the window is destroyed and its webview freed.
            WindowEvent::CloseRequested { .. } => {
                tracker.capture(&handle);
                tracker.save();
            }
            WindowEvent::Focused(false) => tracker.save(),
            _ => {}
        }
    });
    Ok(())
}

fn focus(window: &WebviewWindow) {
    let _ = window.unminimize();
    let _ = window.show();
    let _ = window.set_focus();
}
