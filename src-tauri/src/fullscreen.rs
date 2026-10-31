//! The widget steps aside for full-screen apps (Settings › Widget, on by
//! default): a video in full screen, a game, a slide show. Twice a second the
//! foreground window is checked; while it fills the monitor the widget is on,
//! the widget is hidden, and it comes back when that ends.
//!
//! "Fills the monitor": the window covers the whole monitor (not just the work
//! area, so a maximized window does not count) and either has no title bar and
//! no sizing frame, or Windows itself reports a full-screen app
//! (`SHQueryUserNotificationState`, the check behind muted notifications). The
//! desktop, the taskbar and Skylimit's own windows never count; a full-screen
//! app on another monitor leaves the widget alone.
//!
//! Only the transitions act ([`Tracker`]): the widget hides when full screen
//! starts, if it was showing, and comes back when it ends, if it was hidden
//! for it and the tray's pause is off. Showing it from the tray in between is
//! respected. Coming back never takes the keyboard from the app in front.
//!
//! macOS gives a full-screen app its own Space, where the widget is not, so
//! only Windows needs this.

/// What the watcher does after a check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(not(windows), allow(dead_code))]
pub enum Action {
    None,
    Hide,
    Show,
}

/// Edge-triggered: acts when full screen starts or ends, never in between.
#[derive(Debug, Default)]
#[cfg_attr(not(windows), allow(dead_code))]
pub struct Tracker {
    covered: bool,
    hidden_by_us: bool,
}

#[cfg_attr(not(windows), allow(dead_code))]
impl Tracker {
    pub fn step(&mut self, covered: bool, widget_visible: bool, paused: bool) -> Action {
        if covered == self.covered {
            return Action::None;
        }
        self.covered = covered;
        if covered {
            if widget_visible && !paused {
                self.hidden_by_us = true;
                return Action::Hide;
            }
        } else if std::mem::take(&mut self.hidden_by_us) && !paused {
            return Action::Show;
        }
        Action::None
    }
}

#[cfg(windows)]
pub use watch::spawn;

/// Nothing to watch: full-screen apps cover the widget by themselves.
#[cfg(not(windows))]
pub fn spawn(_app: tauri::AppHandle) {}

#[cfg(windows)]
mod watch {
    use std::time::Duration;

    use tauri::{AppHandle, Manager};
    use windows_sys::Win32::Foundation::{HWND, RECT};
    use windows_sys::Win32::Graphics::Gdi::{GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST};
    use windows_sys::Win32::UI::Shell::{
        SHQueryUserNotificationState, QUERY_USER_NOTIFICATION_STATE, QUNS_BUSY, QUNS_PRESENTATION_MODE,
        QUNS_RUNNING_D3D_FULL_SCREEN,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetClassNameW, GetForegroundWindow, GetWindowLongPtrW, GetWindowRect, GetWindowThreadProcessId, IsIconic,
        IsWindowVisible, SetForegroundWindow, GWL_STYLE, WS_CAPTION, WS_THICKFRAME,
    };

    use super::{Action, Tracker};
    use crate::shell::{self, Shell};
    use crate::windows::{self as wins, WIDGET};

    const TICK: Duration = Duration::from_millis(500);
    /// Desktop and taskbar windows, which cover a monitor without being apps.
    const SHELL_CLASSES: [&str; 4] = ["Progman", "WorkerW", "Shell_TrayWnd", "Shell_SecondaryTrayWnd"];

    pub fn spawn(app: AppHandle) {
        let spawned = std::thread::Builder::new()
            .name("fullscreen-watch".into())
            .spawn(move || run(app));
        if let Err(e) = spawned {
            eprintln!("full-screen watcher not started: {e}");
        }
    }

    fn run(app: AppHandle) {
        let mut tracker = Tracker::default();
        loop {
            std::thread::sleep(TICK);
            // The core is gone: the app is quitting.
            let enabled = match app.state::<Shell>().core() {
                Ok(core) => core.settings().hide_on_fullscreen,
                Err(_) => return,
            };
            let Some(window) = app.get_webview_window(WIDGET) else { continue };
            let Some(widget) = window.hwnd().ok().map(|h| h.0 as HWND) else { continue };
            let covered = enabled && covers_widget_monitor(widget);
            let visible = window.is_visible().unwrap_or(false);
            match tracker.step(covered, visible, shell::is_paused(&app)) {
                Action::Hide => wins::hide_widget(&app),
                Action::Show => {
                    // SAFETY: plain query of the foreground window.
                    let front = unsafe { GetForegroundWindow() };
                    wins::show_widget(&app);
                    give_focus_back(widget, front);
                }
                Action::None => {}
            }
        }
    }

    /// Showing a window may activate it; if the widget took the foreground,
    /// hand it back to the app the user is in.
    fn give_focus_back(widget: HWND, front: HWND) {
        if front.is_null() || front == widget {
            return;
        }
        for _ in 0..4 {
            std::thread::sleep(Duration::from_millis(100));
            // SAFETY: foreground query and a foreground request for a window
            // handle that was valid a moment ago; a stale one is just refused.
            unsafe {
                if GetForegroundWindow() == widget {
                    SetForegroundWindow(front);
                    return;
                }
            }
        }
    }

    /// Does a full-screen app fill the monitor the widget is on?
    fn covers_widget_monitor(widget: HWND) -> bool {
        // SAFETY: read-only queries on window handles from the system; a
        // window that disappears in between makes them fail, not crash.
        unsafe {
            let front = GetForegroundWindow();
            if front.is_null() || front == widget || IsIconic(front) != 0 || IsWindowVisible(front) == 0 {
                return false;
            }
            let mut pid = 0u32;
            GetWindowThreadProcessId(front, &mut pid);
            if pid == std::process::id() || is_shell(front) {
                return false;
            }
            let monitor = MonitorFromWindow(front, MONITOR_DEFAULTTONEAREST);
            if monitor.is_null() || monitor != MonitorFromWindow(widget, MONITOR_DEFAULTTONEAREST) {
                return false;
            }
            let mut info: MONITORINFO = std::mem::zeroed();
            info.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
            let mut rect: RECT = std::mem::zeroed();
            if GetMonitorInfoW(monitor, &mut info) == 0 || GetWindowRect(front, &mut rect) == 0 {
                return false;
            }
            let m = info.rcMonitor;
            if rect.left > m.left || rect.top > m.top || rect.right < m.right || rect.bottom < m.bottom {
                return false;
            }
            let style = GetWindowLongPtrW(front, GWL_STYLE) as u32;
            let borderless = style & WS_CAPTION != WS_CAPTION && style & WS_THICKFRAME == 0;
            borderless || windows_reports_full_screen()
        }
    }

    fn is_shell(window: HWND) -> bool {
        let mut buf = [0u16; 64];
        // SAFETY: the buffer length is passed along.
        let len = unsafe { GetClassNameW(window, buf.as_mut_ptr(), buf.len() as i32) };
        let class = String::from_utf16_lossy(&buf[..len.max(0) as usize]);
        SHELL_CLASSES.contains(&class.as_str())
    }

    fn windows_reports_full_screen() -> bool {
        let mut state: QUERY_USER_NOTIFICATION_STATE = 0;
        // SAFETY: one out parameter.
        let ok = unsafe { SHQueryUserNotificationState(&mut state) } >= 0;
        ok && matches!(state, QUNS_BUSY | QUNS_RUNNING_D3D_FULL_SCREEN | QUNS_PRESENTATION_MODE)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hides_and_returns_only_on_the_transitions() {
        let mut t = Tracker::default();
        assert_eq!(t.step(false, true, false), Action::None);
        assert_eq!(t.step(true, true, false), Action::Hide);
        // Still covered (and now hidden): nothing more.
        assert_eq!(t.step(true, false, false), Action::None);
        assert_eq!(t.step(false, false, false), Action::Show);
        assert_eq!(t.step(false, true, false), Action::None);
    }

    #[test]
    fn respects_what_the_user_did_meanwhile() {
        // Hidden from the tray before: full screen leaves it hidden after.
        let mut t = Tracker::default();
        assert_eq!(t.step(true, false, false), Action::None);
        assert_eq!(t.step(false, false, false), Action::None);

        // Shown from the tray during full screen: not hidden again.
        let mut t = Tracker::default();
        assert_eq!(t.step(true, true, false), Action::Hide);
        assert_eq!(t.step(true, true, false), Action::None);

        // Paused during full screen: stays hidden after it.
        let mut t = Tracker::default();
        assert_eq!(t.step(true, true, false), Action::Hide);
        assert_eq!(t.step(false, false, true), Action::None);
        assert_eq!(t.step(true, false, true), Action::None);
        assert_eq!(t.step(false, false, false), Action::None);
    }
}
