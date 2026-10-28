//! Notification-area icon: a small gauge drawn in code whose color is
//! `Snapshot.tray` (the core decides; the tray only renders), a tooltip with
//! the headline numbers and the Czech menu.
//!
//! Adapted from BFocus (`src-tauri/src/tray.rs`): icon pixels are computed
//! here, so no PNG per state ships with the app.

use std::sync::Mutex;

use tauri::image::Image;
use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, Wry};
use usage_core::{Currency, LimitsState, Provider, Snapshot, Tray, WindowKind, WindowState};

use crate::{shell, windows};

const SIZE: u32 = 32;
/// Windows truncates tooltips at 127 UTF-16 units.
const TOOLTIP_MAX: usize = 120;
const NBSP: char = '\u{a0}';

pub struct TrayHandles {
    tray: TrayIcon<Wry>,
    click_through: CheckMenuItem<Wry>,
    pause: CheckMenuItem<Wry>,
    cache: Mutex<Cache>,
}

#[derive(Default)]
struct Cache {
    icon: Option<Tray>,
    tooltip: String,
}

pub fn create(app: &AppHandle, click_through: bool) -> tauri::Result<()> {
    let widget = MenuItem::with_id(app, "widget", "Zobrazit/skrýt widget", true, None::<&str>)?;
    let click = CheckMenuItem::with_id(app, "click_through", "Průchod kliknutí", true, click_through, None::<&str>)?;
    let open = MenuItem::with_id(app, "app", "Otevřít aplikaci", true, None::<&str>)?;
    let feedback = MenuItem::with_id(app, "feedback", "Poslat zpětnou vazbu…", true, None::<&str>)?;
    let pause = CheckMenuItem::with_id(app, "pause", "Pauza", true, false, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Konec", true, None::<&str>)?;
    let menu = Menu::with_items(
        app,
        &[
            &widget,
            &click,
            &open,
            &feedback,
            &PredefinedMenuItem::separator(app)?,
            &pause,
            &PredefinedMenuItem::separator(app)?,
            &quit,
        ],
    )?;
    let tray = TrayIconBuilder::with_id("main")
        .icon(draw(Tray::Grey))
        .tooltip("Skylimit · načítám…")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "widget" => windows::toggle_widget(app),
            "click_through" => {
                let on = app
                    .try_state::<TrayHandles>()
                    .and_then(|h| h.click_through.is_checked().ok())
                    .unwrap_or(false);
                if let Err(e) = shell::set_click_through(app, on) {
                    eprintln!("click-through from tray failed: {e}");
                }
            }
            "app" => {
                if let Err(e) = windows::open_app(app) {
                    eprintln!("cannot open the app window: {e}");
                }
            }
            "feedback" => {
                if let Err(e) = windows::open_feedback(app) {
                    eprintln!("cannot open the feedback dialog: {e}");
                }
            }
            "pause" => {
                let on = app
                    .try_state::<TrayHandles>()
                    .and_then(|h| h.pause.is_checked().ok())
                    .unwrap_or(false);
                windows::set_paused(app, on);
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                windows::toggle_widget(tray.app_handle());
            }
        })
        .build(app)?;
    app.manage(TrayHandles {
        tray,
        click_through: click,
        pause,
        cache: Mutex::new(Cache::default()),
    });
    Ok(())
}

/// Bring icon and tooltip in line with a snapshot. Only touches what changed.
pub fn update(app: &AppHandle, snap: &Snapshot) {
    let Some(handles) = app.try_state::<TrayHandles>() else {
        return;
    };
    let paused = shell::is_paused(app);
    let icon = if paused { Tray::Mono } else { snap.tray };
    let tooltip = tooltip(snap, paused);

    // Decide under the lock, call the tray after releasing it: tray methods
    // wait for the main thread, which may itself be waiting for this lock.
    let (icon_change, tooltip_change) = {
        let mut cache = handles.cache.lock().unwrap_or_else(|e| e.into_inner());
        let icon_change = (cache.icon != Some(icon)).then_some(icon);
        cache.icon = Some(icon);
        let tooltip_change = (cache.tooltip != tooltip).then(|| tooltip.clone());
        cache.tooltip = tooltip;
        (icon_change, tooltip_change)
    };
    if let Some(icon) = icon_change {
        let _ = handles.tray.set_icon(Some(draw(icon)));
    }
    if let Some(text) = tooltip_change {
        let _ = handles.tray.set_tooltip(Some(&text));
    }
}

/// Re-render from the core's current snapshot (after pause toggles).
pub fn refresh(app: &AppHandle) {
    if let Some(snap) = shell::current_snapshot(app) {
        update(app, &snap);
    }
}

pub fn set_click_through_checked(app: &AppHandle, on: bool) {
    if let Some(handles) = app.try_state::<TrayHandles>() {
        let _ = handles.click_through.set_checked(on);
    }
}

pub fn set_pause_checked(app: &AppHandle, on: bool) {
    if let Some(handles) = app.try_state::<TrayHandles>() {
        let _ = handles.pause.set_checked(on);
    }
}

// ---------------------------------------------------------------- tooltip ---

/// "Skylimit · Codex 5h 12 %, týden 40 % · 1 201 €"
fn tooltip(snap: &Snapshot, paused: bool) -> String {
    if paused {
        return "Skylimit · pozastaveno".into();
    }
    if snap.month.is_empty() {
        return "Skylimit · načítám…".into();
    }
    let mut parts = vec!["Skylimit".to_string()];
    for limits in &snap.limits {
        if limits.state == LimitsState::Off {
            continue;
        }
        let name = provider_name(limits.provider);
        let windows: Vec<String> = limits
            .windows
            .iter()
            .filter_map(|w| {
                let label = match w.kind {
                    WindowKind::FiveHour => "5h",
                    WindowKind::Week => "týden",
                    _ => return None,
                };
                let percent = w.percent?;
                let approx = match w.state {
                    WindowState::Known => "",
                    WindowState::Stale => "~",
                    _ => return None,
                };
                Some(format!("{label} {approx}{}{NBSP}%", percent.round() as i64))
            })
            .collect();
        if windows.is_empty() {
            parts.push(format!("{name} ?"));
        } else {
            parts.push(format!("{name} {}", windows.join(", ")));
        }
    }
    parts.push(money(snap.total.display, snap.currency));
    truncate(parts.join(" · "), TOOLTIP_MAX)
}

fn provider_name(provider: Provider) -> &'static str {
    match provider {
        Provider::Claude => "Claude",
        Provider::Codex => "Codex",
    }
}

/// Same rules as the UI's `formatMoney`: "1 201 €", "23 436 Kč", "8,40 €".
fn money(value: f64, currency: Currency) -> String {
    let symbol = match currency {
        Currency::Eur => "€",
        Currency::Czk => "Kč",
    };
    let value = if value.is_finite() { value } else { 0.0 };
    let abs = value.abs();
    let sign = if value < 0.0 { "-" } else { "" };
    let number = if abs == 0.0 || abs >= 10.0 {
        group(abs.round() as u64)
    } else {
        let cents = (abs * 100.0).round() as u64;
        format!("{},{:02}", group(cents / 100), cents % 100)
    };
    format!("{sign}{number}{NBSP}{symbol}")
}

fn group(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(NBSP);
        }
        out.push(c);
    }
    out
}

fn truncate(text: String, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text;
    }
    let mut cut: String = text.chars().take(max_chars - 1).collect();
    cut.push('…');
    cut
}

// ------------------------------------------------------------------- icon ---

struct Look {
    ring: [u8; 3],
    ring_alpha: f32,
    dot: Option<[u8; 3]>,
}

fn mix(c: [u8; 3], toward: [u8; 3], t: f32) -> [u8; 3] {
    let m = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * t).round() as u8;
    [m(c[0], toward[0]), m(c[1], toward[1]), m(c[2], toward[2])]
}

/// Colors match the UI status tokens (`--ok`, `--warn`, `--crit`).
fn look(state: Tray) -> Look {
    const GREEN: [u8; 3] = [0x5f, 0xb9, 0x7e];
    const ORANGE: [u8; 3] = [0xe2, 0xa6, 0x45];
    const RED: [u8; 3] = [0xe0, 0x64, 0x5a];
    const GREY: [u8; 3] = [0x8a, 0x93, 0xa2];
    const MONO: [u8; 3] = [0xc9, 0xce, 0xd6];
    const MUTE: [u8; 3] = [0x6b, 0x72, 0x80];
    let solid = |c: [u8; 3]| Look {
        ring: c,
        ring_alpha: 1.0,
        dot: Some(c),
    };
    // Stale: same hue, muted and slightly see-through.
    let dim = |c: [u8; 3]| {
        let c = mix(c, MUTE, 0.5);
        Look {
            ring: c,
            ring_alpha: 0.8,
            dot: Some(c),
        }
    };
    match state {
        Tray::Green => solid(GREEN),
        Tray::Orange => solid(ORANGE),
        Tray::Red => solid(RED),
        Tray::GreenDim => dim(GREEN),
        Tray::OrangeDim => dim(ORANGE),
        Tray::RedDim => dim(RED),
        // No provider has a value: an empty grey gauge ("don't know").
        Tray::Grey => Look {
            ring: GREY,
            ring_alpha: 1.0,
            dot: None,
        },
        // Limits deliberately off everywhere (or paused): neutral, no status color.
        Tray::Mono => solid(MONO),
    }
}

/// Gauge glyph: an arc open at the bottom plus a center dot.
fn draw(state: Tray) -> Image<'static> {
    let look = look(state);
    let size = SIZE as f32;
    let cx = (size - 1.0) / 2.0;
    let cy = cx;
    let outer = size * 0.47;
    let inner = size * 0.33;
    let mid = (outer + inner) / 2.0;
    let dot_r = size * 0.17;
    // Half-width of the bottom gap, radians from straight down.
    let gap = 0.75f32;
    let cover = |d: f32, lo: f32, hi: f32| ((hi + 0.5 - d).clamp(0.0, 1.0) * (d - lo + 0.5).clamp(0.0, 1.0)).clamp(0.0, 1.0);
    let mut rgba = vec![0u8; (SIZE * SIZE * 4) as usize];
    for y in 0..SIZE {
        for x in 0..SIZE {
            let dx = x as f32 - cx;
            let dy = y as f32 - cy;
            let d = (dx * dx + dy * dy).sqrt();
            // Angle away from "straight down" (0 = bottom, PI = top).
            let from_bottom = dx.atan2(dy).abs();
            let arc_a = ((from_bottom - gap) * mid + 0.5).clamp(0.0, 1.0);
            let ring_a = cover(d, inner, outer) * arc_a * look.ring_alpha;
            let dot_a = match look.dot {
                Some(_) => cover(d, -1.0, dot_r) * look.ring_alpha,
                None => 0.0,
            };
            let (mut r, mut g, mut b, mut a) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
            let mut over = |c: [u8; 3], alpha: f32| {
                if alpha <= 0.0 {
                    return;
                }
                let out_a = alpha + a * (1.0 - alpha);
                r = (c[0] as f32 * alpha + r * a * (1.0 - alpha)) / out_a;
                g = (c[1] as f32 * alpha + g * a * (1.0 - alpha)) / out_a;
                b = (c[2] as f32 * alpha + b * a * (1.0 - alpha)) / out_a;
                a = out_a;
            };
            over(look.ring, ring_a);
            if let Some(c) = look.dot {
                over(c, dot_a);
            }
            let i = ((y * SIZE + x) * 4) as usize;
            rgba[i] = r.round() as u8;
            rgba[i + 1] = g.round() as u8;
            rgba[i + 2] = b.round() as u8;
            rgba[i + 3] = (a * 255.0).round() as u8;
        }
    }
    Image::new_owned(rgba, SIZE, SIZE)
}

#[cfg(test)]
mod tests {
    use super::*;
    use usage_core::{LimitWindow, ProviderLimits};

    const ALL: [Tray; 8] = [
        Tray::Green,
        Tray::Orange,
        Tray::Red,
        Tray::GreenDim,
        Tray::OrangeDim,
        Tray::RedDim,
        Tray::Grey,
        Tray::Mono,
    ];

    fn px(img: &Image<'_>, x: u32, y: u32) -> [u8; 4] {
        let i = ((y * SIZE + x) * 4) as usize;
        let p = &img.rgba()[i..i + 4];
        [p[0], p[1], p[2], p[3]]
    }

    #[test]
    fn icons_have_expected_size_and_transparent_corners() {
        for state in ALL {
            let img = draw(state);
            assert_eq!(img.width(), SIZE);
            assert_eq!(img.height(), SIZE);
            assert_eq!(img.rgba().len(), (SIZE * SIZE * 4) as usize);
            assert_eq!(px(&img, 0, 0)[3], 0, "{state:?}");
            // The arc is open at the bottom.
            assert_eq!(px(&img, SIZE / 2, SIZE - 2)[3], 0, "{state:?}");
            // ...and closed at the top.
            assert!(px(&img, SIZE / 2, 2)[3] > 150, "{state:?}");
        }
    }

    #[test]
    fn grey_is_hollow_and_mono_is_not() {
        let center = (SIZE / 2, SIZE / 2);
        assert_eq!(px(&draw(Tray::Grey), center.0, center.1)[3], 0);
        assert!(px(&draw(Tray::Mono), center.0, center.1)[3] > 200);
    }

    #[test]
    fn every_state_looks_different() {
        let images: Vec<Vec<u8>> = ALL.iter().map(|s| draw(*s).rgba().to_vec()).collect();
        for i in 0..images.len() {
            for j in i + 1..images.len() {
                assert_ne!(images[i], images[j], "{:?} vs {:?}", ALL[i], ALL[j]);
            }
        }
    }

    #[test]
    fn money_matches_ui_format() {
        assert_eq!(money(1201.4, Currency::Eur), "1\u{a0}201\u{a0}€");
        assert_eq!(money(23436.0, Currency::Czk), "23\u{a0}436\u{a0}Kč");
        assert_eq!(money(8.4, Currency::Eur), "8,40\u{a0}€");
        assert_eq!(money(0.0, Currency::Eur), "0\u{a0}€");
        assert_eq!(money(930.0, Currency::Eur), "930\u{a0}€");
        assert_eq!(money(1_234_567.0, Currency::Czk), "1\u{a0}234\u{a0}567\u{a0}Kč");
    }

    fn window(kind: WindowKind, percent: Option<f64>, state: WindowState) -> LimitWindow {
        LimitWindow {
            kind,
            label: String::new(),
            percent,
            resets_at: None,
            fetched_at: None,
            state,
        }
    }

    fn snapshot() -> Snapshot {
        let mut snap = Snapshot::empty(0, Currency::Eur);
        snap.month = "2026-10".into();
        snap.total.display = 1201.0;
        snap.limits = vec![
            ProviderLimits {
                provider: Provider::Claude,
                state: LimitsState::Off,
                governing: None,
                windows: vec![],
                note: None,
            },
            ProviderLimits {
                provider: Provider::Codex,
                state: LimitsState::Known,
                governing: Some(40.0),
                windows: vec![
                    window(WindowKind::FiveHour, Some(12.4), WindowState::Known),
                    window(WindowKind::Week, Some(40.0), WindowState::Stale),
                    window(WindowKind::WeekModel, Some(70.0), WindowState::Known),
                ],
                note: None,
            },
        ];
        snap
    }

    #[test]
    fn tooltip_shows_codex_windows_and_total() {
        assert_eq!(
            tooltip(&snapshot(), false),
            "Skylimit · Codex 5h 12\u{a0}%, týden ~40\u{a0}% · 1\u{a0}201\u{a0}€"
        );
    }

    #[test]
    fn tooltip_marks_unknown_limits_pause_and_loading() {
        let mut snap = snapshot();
        snap.limits[1].state = LimitsState::Unknown;
        snap.limits[1].windows = vec![window(WindowKind::FiveHour, None, WindowState::AfterReset)];
        assert_eq!(tooltip(&snap, false), "Skylimit · Codex ? · 1\u{a0}201\u{a0}€");
        assert_eq!(tooltip(&snap, true), "Skylimit · pozastaveno");
        assert_eq!(tooltip(&Snapshot::empty(0, Currency::Eur), false), "Skylimit · načítám…");
    }

    #[test]
    fn long_tooltips_are_cut() {
        let cut = truncate("x".repeat(300), TOOLTIP_MAX);
        assert_eq!(cut.chars().count(), TOOLTIP_MAX);
        assert!(cut.ends_with('…'));
    }
}
