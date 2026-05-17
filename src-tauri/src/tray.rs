//! System tray, global shortcuts, and end-of-day notification (desktop only).

use std::sync::OnceLock;
use std::time::Duration;

use chrono::Timelike;
use tauri::image::Image;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::tray::{TrayIcon, TrayIconBuilder};
use tauri::{AppHandle, Manager, Wry};
use tauri_plugin_global_shortcut::{Code, Shortcut, ShortcutEvent, ShortcutState};
use tauri_plugin_notification::NotificationExt;

use crate::analytics::dashboard;
use crate::db::queries;
use crate::tracker::{engagement, window};
use tauri_plugin_sql::DbInstances;

/// Tray JE icon bytes (`include_bytes!`); `icon_as_template` omitted for macOS compatibility.
fn tray_je_icon() -> Image<'static> {
    Image::from_bytes(include_bytes!(
        "../icons/tray/JETemplate@2x.png"
    ))
    .expect("JE tray template PNG")
    .to_owned()
}

pub struct TrayUi {
    pub tray: TrayIcon<Wry>,
    pub status: MenuItem<Wry>,
    pub focus: MenuItem<Wry>,
    pub pause_menu: Submenu<Wry>,
    pub resume: MenuItem<Wry>,
}

static TRAY_UI: OnceLock<TrayUi> = OnceLock::new();

fn toggle_main_window(app: &AppHandle<Wry>) {
    let Some(w) = app.get_webview_window("main") else {
        return;
    };
    let visible = w.is_visible().unwrap_or(true);
    if visible {
        let _ = w.hide();
    } else {
        let _ = w.show();
        let _ = w.set_focus();
    }
}

fn toggle_pause(app: &AppHandle<Wry>) {
    let h = app.clone();
    tauri::async_runtime::spawn(async move {
        if window::is_foreground_paused() {
            window::resume_foreground_tracking(h);
        } else {
            window::pause_foreground_tracking(&h).await;
        }
    });
}

pub fn on_global_shortcut(app: &AppHandle<Wry>, shortcut: &Shortcut, event: ShortcutEvent) {
    if event.state != ShortcutState::Pressed {
        return;
    }

    if shortcut.key == Code::KeyJ {
        toggle_main_window(app);
    } else if shortcut.key == Code::KeyP {
        toggle_pause(app);
    }
}

async fn tray_refresh_tick(app: AppHandle<Wry>) {
    let Some(ui) = TRAY_UI.get() else {
        return;
    };

    let _ = ui.status.set_text(engagement::tray_status_label());

    let focus = dashboard::get_today_overview(&app)
        .await
        .map(|o| o.focus_score)
        .unwrap_or(0);
    let _ = ui
        .focus
        .set_text(format!("Today's Focus Score: {focus}"));

    let tip = format!("Jadon's Excuses — {}", engagement::tray_status_label());
    let _ = ui.tray.set_tooltip(Some(tip));

    let paused = window::is_foreground_paused();
    let _ = ui.resume.set_enabled(paused);
    let _ = ui.pause_menu.set_enabled(!paused);
}

async fn daily_summary_tick(app: AppHandle<Wry>) {
    use chrono::Local;

    static LAST_DAY: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

    let instances = app.state::<DbInstances>();

    let enabled = match queries::get_app_setting(&instances, "daily_summary_enabled").await {
        Ok(Some(v)) => v != "false" && v != "0",
        Ok(None) => true,
        Err(_) => true,
    };
    if !enabled {
        return;
    }

    let hour: u32 = queries::get_app_setting(&instances, "daily_summary_hour")
        .await
        .ok()
        .flatten()
        .and_then(|s| s.parse().ok())
        .unwrap_or(18);
    let minute: u32 = queries::get_app_setting(&instances, "daily_summary_minute")
        .await
        .ok()
        .flatten()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);

    let now = Local::now();
    if now.hour() != hour || now.minute() != minute {
        return;
    }

    let day = now.format("%Y-%m-%d").to_string();
    {
        let mut g = LAST_DAY.lock().unwrap();
        if g.as_deref() == Some(day.as_str()) {
            return;
        }
        *g = Some(day.clone());
    }

    let overview = match dashboard::get_today_overview(&app).await {
        Ok(o) => o,
        Err(_) => return,
    };
    let streak_sec = dashboard::longest_streak_seconds_today_utc(&app).await;
    let streak_m = (streak_sec / 60).max(1);

    let body = format!(
        "Focus Score {}, {} switches, longest streak {}m. Open dashboard.",
        overview.focus_score, overview.switch_count, streak_m
    );

    let _ = app
        .notification()
        .builder()
        .title("Your day in numbers")
        .body(body)
        .show();
}

pub fn setup(app: &AppHandle<Wry>) -> tauri::Result<()> {
    let status = MenuItem::with_id(
        app,
        "tray_status",
        "● … | Score: —",
        false,
        Option::<&str>::None,
    )?;
    let focus = MenuItem::with_id(
        app,
        "tray_focus",
        "Today's Focus Score: —",
        false,
        Option::<&str>::None,
    )?;

    let show = MenuItem::with_id(
        app,
        "tray_show",
        "Show Dashboard",
        true,
        Option::<&str>::None,
    )?;

    let pause_15 = MenuItem::with_id(
        app,
        "pause_15",
        "15 minutes",
        true,
        Option::<&str>::None,
    )?;
    let pause_30 = MenuItem::with_id(
        app,
        "pause_30",
        "30 minutes",
        true,
        Option::<&str>::None,
    )?;
    let pause_60 = MenuItem::with_id(
        app,
        "pause_60",
        "1 hour",
        true,
        Option::<&str>::None,
    )?;
    let pause_manual = MenuItem::with_id(
        app,
        "pause_manual",
        "Until I resume",
        true,
        Option::<&str>::None,
    )?;

    let pause_menu = Submenu::with_id_and_items(
        app,
        "tray_pause_menu",
        "Pause tracking",
        true,
        &[&pause_15, &pause_30, &pause_60, &pause_manual],
    )?;

    let resume = MenuItem::with_id(
        app,
        "tray_resume",
        "Resume tracking",
        true,
        Option::<&str>::None,
    )?;
    let _ = resume.set_enabled(false);

    let quit = MenuItem::with_id(app, "tray_quit", "Quit Jadon's Excuses", true, Option::<&str>::None)?;

    let menu = Menu::with_items(
        app,
        &[
            &show,
            &PredefinedMenuItem::separator(app)?,
            &status,
            &focus,
            &PredefinedMenuItem::separator(app)?,
            &pause_menu,
            &resume,
            &PredefinedMenuItem::separator(app)?,
            &quit,
        ],
    )?;

    let tray = TrayIconBuilder::new()
        .menu(&menu)
        .icon(tray_je_icon())
        .show_menu_on_left_click(true)
        .on_menu_event(|app, event| {
            let id = event.id.as_ref();
            match id {
                "tray_show" => {
                    if let Some(w) = app.get_webview_window("main") {
                        let _ = w.unminimize();
                        let _ = w.show();
                        let _ = w.set_focus();
                    }
                }
                "pause_15" => {
                    let h = app.clone();
                    tauri::async_runtime::spawn(async move {
                        window::pause_foreground_tracking(&h).await;
                        window::schedule_resume_foreground(h.clone(), Duration::from_secs(15 * 60));
                    });
                }
                "pause_30" => {
                    let h = app.clone();
                    tauri::async_runtime::spawn(async move {
                        window::pause_foreground_tracking(&h).await;
                        window::schedule_resume_foreground(h.clone(), Duration::from_secs(30 * 60));
                    });
                }
                "pause_60" => {
                    let h = app.clone();
                    tauri::async_runtime::spawn(async move {
                        window::pause_foreground_tracking(&h).await;
                        window::schedule_resume_foreground(h.clone(), Duration::from_secs(60 * 60));
                    });
                }
                "pause_manual" => {
                    let h = app.clone();
                    tauri::async_runtime::spawn(async move {
                        window::pause_foreground_tracking(&h).await;
                    });
                }
                "tray_resume" => {
                    window::resume_foreground_tracking(app.clone());
                }
                "tray_quit" => {
                    app.exit(0);
                }
                _ => {}
            }
        })
        .build(app)?;

    let ui = TrayUi {
        tray: tray.clone(),
        status,
        focus,
        pause_menu,
        resume,
    };
    let _ = TRAY_UI.set(ui);

    let h = app.clone();
    tauri::async_runtime::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(5));
        loop {
            interval.tick().await;
            tray_refresh_tick(h.clone()).await;
        }
    });

    let h2 = app.clone();
    tauri::async_runtime::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(60));
        loop {
            interval.tick().await;
            daily_summary_tick(h2.clone()).await;
        }
    });

    Ok(())
}
