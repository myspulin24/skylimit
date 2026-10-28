//! Remembers where the widget and the app window were between runs.
//!
//! On Windows this uses the placement API (`GetWindowPlacement` /
//! `SetWindowPlacement`): it reports the normal (restored) rectangle even
//! while a window is maximized or minimized, and when the saved position is no
//! longer on any monitor Windows moves the window back on screen. Also hosts
//! the `WS_EX_TOOLWINDOW` helper the widget needs, since both are thin Win32
//! wrappers around a window handle. macOS and Linux use Tauri's own position
//! and size, and skip a saved rectangle that is on no monitor any more.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tauri::{Runtime, WebviewWindow};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Placement {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
    pub maximized: bool,
}

impl Placement {
    /// Rejects corrupt or absurd values instead of trusting the file.
    fn plausible(&self) -> bool {
        let width = i64::from(self.right) - i64::from(self.left);
        let height = i64::from(self.bottom) - i64::from(self.top);
        let on_desktop = |v: i32| (-100_000..=100_000).contains(&v);
        (200..=32_000).contains(&width)
            && (200..=32_000).contains(&height)
            && on_desktop(self.left)
            && on_desktop(self.top)
    }

    /// Same top-left corner, different size (the widget keeps its own size).
    pub fn with_size(self, width: i32, height: i32) -> Placement {
        Placement {
            right: self.left.saturating_add(width),
            bottom: self.top.saturating_add(height),
            maximized: false,
            ..self
        }
    }
}

fn load(path: &Path) -> Option<Placement> {
    let text = std::fs::read_to_string(path).ok()?;
    serde_json::from_str::<Placement>(&text)
        .ok()
        .filter(Placement::plausible)
}

fn store(path: &Path, placement: &Placement) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(placement)?)?;
    std::fs::rename(&tmp, path)
}

/// Latest placement of one window, written to disk when it matters
/// (focus lost, hidden, closed, app exit).
pub struct WindowTracker {
    file: PathBuf,
    last: Mutex<Option<Placement>>,
    saved: Mutex<Option<Placement>>,
}

impl WindowTracker {
    pub fn new(file: PathBuf) -> Self {
        WindowTracker {
            file,
            last: Mutex::new(None),
            saved: Mutex::new(None),
        }
    }

    /// Placement stored by a previous run, if it is plausible.
    pub fn stored(&self) -> Option<Placement> {
        let placement = load(&self.file);
        if let Ok(mut saved) = self.saved.lock() {
            *saved = placement;
        }
        placement
    }

    pub fn capture<R: Runtime>(&self, window: &WebviewWindow<R>) {
        if let Some(p) = native::get(window) {
            if let Ok(mut last) = self.last.lock() {
                *last = Some(p);
            }
        }
    }

    /// Write the last known placement, skipping the disk when nothing moved.
    pub fn save(&self) {
        let Some(placement) = self.last.lock().ok().and_then(|l| *l) else {
            return;
        };
        let Ok(mut saved) = self.saved.lock() else {
            return;
        };
        if *saved == Some(placement) {
            return;
        }
        match store(&self.file, &placement) {
            Ok(()) => *saved = Some(placement),
            Err(e) => eprintln!("cannot save window placement {}: {e}", self.file.display()),
        }
    }
}

/// Apply a placement. `keep_hidden` moves a hidden window without showing it
/// (the caller shows it through Tauri so its own visibility state stays right).
pub fn apply<R: Runtime>(window: &WebviewWindow<R>, placement: &Placement, keep_hidden: bool) -> bool {
    native::set(window, placement, keep_hidden)
}

/// Make the window a tool window: not in Alt+Tab, not on the taskbar.
/// Must run on the main thread (the window's owner thread).
pub fn make_tool_window<R: Runtime>(window: &WebviewWindow<R>) {
    native::make_tool_window(window);
}

/// Current extended style contains `WS_EX_TOOLWINDOW` (diagnostics and tests).
#[allow(dead_code)]
pub fn is_tool_window<R: Runtime>(window: &WebviewWindow<R>) -> bool {
    native::is_tool_window(window)
}

#[cfg(windows)]
mod native {
    use super::Placement;
    use tauri::{Runtime, WebviewWindow};
    use windows_sys::Win32::Foundation::{HWND, POINT, RECT};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetWindowLongPtrW, GetWindowPlacement, SetWindowLongPtrW, SetWindowPlacement, SetWindowPos,
        GWL_EXSTYLE, SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SW_HIDE,
        SW_SHOWMAXIMIZED, SW_SHOWMINIMIZED, SW_SHOWNORMAL, WINDOWPLACEMENT, WPF_RESTORETOMAXIMIZED,
        WS_EX_APPWINDOW, WS_EX_TOOLWINDOW,
    };

    fn hwnd<R: Runtime>(window: &WebviewWindow<R>) -> Option<HWND> {
        window.hwnd().ok().map(|h| h.0 as HWND)
    }

    fn blank() -> WINDOWPLACEMENT {
        WINDOWPLACEMENT {
            length: std::mem::size_of::<WINDOWPLACEMENT>() as u32,
            flags: 0,
            showCmd: 0,
            ptMinPosition: POINT { x: 0, y: 0 },
            ptMaxPosition: POINT { x: 0, y: 0 },
            rcNormalPosition: RECT {
                left: 0,
                top: 0,
                right: 0,
                bottom: 0,
            },
        }
    }

    pub fn get<R: Runtime>(window: &WebviewWindow<R>) -> Option<Placement> {
        let hwnd = hwnd(window)?;
        let mut wp = blank();
        // SAFETY: `hwnd` belongs to a live window of this process and
        // `wp.length` is initialised as the API requires.
        if unsafe { GetWindowPlacement(hwnd, &mut wp) } == 0 {
            return None;
        }
        let r = wp.rcNormalPosition;
        let maximized = wp.showCmd == SW_SHOWMAXIMIZED as u32
            || (wp.showCmd == SW_SHOWMINIMIZED as u32 && wp.flags & WPF_RESTORETOMAXIMIZED != 0);
        Some(Placement {
            left: r.left,
            top: r.top,
            right: r.right,
            bottom: r.bottom,
            maximized,
        })
    }

    pub fn set<R: Runtime>(window: &WebviewWindow<R>, p: &Placement, keep_hidden: bool) -> bool {
        let Some(hwnd) = hwnd(window) else {
            return false;
        };
        let mut wp = blank();
        wp.showCmd = if keep_hidden {
            SW_HIDE
        } else if p.maximized {
            SW_SHOWMAXIMIZED
        } else {
            SW_SHOWNORMAL
        } as u32;
        wp.rcNormalPosition = RECT {
            left: p.left,
            top: p.top,
            right: p.right,
            bottom: p.bottom,
        };
        // SAFETY: as in `get`; the structure is fully initialised.
        unsafe { SetWindowPlacement(hwnd, &wp) != 0 }
    }

    pub fn make_tool_window<R: Runtime>(window: &WebviewWindow<R>) {
        let Some(hwnd) = hwnd(window) else {
            return;
        };
        // SAFETY: plain style read/write on a live window owned by this thread.
        unsafe {
            let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
            let wanted = (ex | WS_EX_TOOLWINDOW as isize) & !(WS_EX_APPWINDOW as isize);
            if wanted != ex {
                SetWindowLongPtrW(hwnd, GWL_EXSTYLE, wanted);
                SetWindowPos(
                    hwnd,
                    std::ptr::null_mut(),
                    0,
                    0,
                    0,
                    0,
                    SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_FRAMECHANGED,
                );
            }
        }
    }

    pub fn is_tool_window<R: Runtime>(window: &WebviewWindow<R>) -> bool {
        let Some(hwnd) = hwnd(window) else {
            return false;
        };
        // SAFETY: read-only style query on a live window.
        unsafe { GetWindowLongPtrW(hwnd, GWL_EXSTYLE) & WS_EX_TOOLWINDOW as isize != 0 }
    }
}

#[cfg(not(windows))]
mod native {
    use super::Placement;
    use tauri::{PhysicalPosition, PhysicalSize, Runtime, WebviewWindow};

    /// Outer position plus inner size (what `set_size` takes back). There is
    /// no separate "restored" rectangle here, so a maximized window is saved
    /// at its maximized size together with the flag.
    pub fn get<R: Runtime>(window: &WebviewWindow<R>) -> Option<Placement> {
        if window.is_minimized().unwrap_or(false) {
            return None;
        }
        let pos = window.outer_position().ok()?;
        let size = window.inner_size().ok()?;
        Some(Placement {
            left: pos.x,
            top: pos.y,
            right: pos.x.saturating_add(i32::try_from(size.width).ok()?),
            bottom: pos.y.saturating_add(i32::try_from(size.height).ok()?),
            maximized: window.is_maximized().unwrap_or(false),
        })
    }

    pub fn set<R: Runtime>(window: &WebviewWindow<R>, p: &Placement, _keep_hidden: bool) -> bool {
        // A rectangle left on a monitor that is gone would open off screen.
        let on_screen = window
            .available_monitors()
            .map(|monitors| {
                monitors.iter().any(|m| {
                    let (mp, ms) = (m.position(), m.size());
                    let right = mp.x.saturating_add(ms.width as i32);
                    let bottom = mp.y.saturating_add(ms.height as i32);
                    p.left < right && p.right > mp.x && p.top < bottom && p.bottom > mp.y
                })
            })
            .unwrap_or(true);
        if !on_screen {
            return false;
        }
        let width = (p.right - p.left).max(1) as u32;
        let height = (p.bottom - p.top).max(1) as u32;
        let moved = window.set_size(PhysicalSize::new(width, height)).is_ok()
            && window.set_position(PhysicalPosition::new(p.left, p.top)).is_ok();
        if moved && p.maximized {
            let _ = window.maximize();
        }
        moved
    }

    pub fn make_tool_window<R: Runtime>(_window: &WebviewWindow<R>) {}

    pub fn is_tool_window<R: Runtime>(_window: &WebviewWindow<R>) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Placement {
        Placement {
            left: 140,
            top: 90,
            right: 1251,
            bottom: 867,
            maximized: true,
        }
    }

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ai-usage-window-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn round_trips_through_the_file() {
        let dir = temp_dir("roundtrip");
        let file = dir.join("app-window.json");
        store(&file, &sample()).unwrap();
        assert_eq!(load(&file), Some(sample()));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn rejects_implausible_placements() {
        let tiny = Placement {
            right: 150,
            ..sample()
        };
        let inverted = Placement {
            right: 0,
            bottom: 0,
            ..sample()
        };
        let far = Placement {
            left: 5_000_000,
            right: 5_001_000,
            ..sample()
        };
        for p in [tiny, inverted, far] {
            assert!(!p.plausible(), "{p:?}");
        }
        assert!(sample().plausible());
    }

    #[test]
    fn ignores_corrupt_files() {
        let dir = temp_dir("bad");
        let file = dir.join("widget-window.json");
        std::fs::write(&file, b"{ not json").unwrap();
        assert_eq!(load(&file), None);
        assert_eq!(load(&dir.join("missing.json")), None);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn with_size_keeps_the_corner() {
        let p = sample().with_size(360, 480);
        assert_eq!((p.left, p.top, p.right, p.bottom), (140, 90, 500, 570));
        assert!(!p.maximized);
    }

    #[test]
    fn tracker_saves_only_changes() {
        let dir = temp_dir("tracker");
        let file = dir.join("widget-window.json");
        let tracker = WindowTracker::new(file.clone());
        tracker.save(); // nothing captured yet
        assert!(!file.exists());
        *tracker.last.lock().unwrap() = Some(sample());
        tracker.save();
        assert_eq!(tracker.stored(), Some(sample()));
        std::fs::remove_file(&file).unwrap();
        tracker.save(); // unchanged since the last save: no write
        assert!(!file.exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
