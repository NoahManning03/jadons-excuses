#[allow(dead_code)]
mod analytics;
mod commands;
mod db;
// `#[allow(dead_code)]` is intentional — submodules expose surface area
// (browser_bridge, BROWSER_APPS, engagement::current_event_id) that the
// next two roadmap steps wire up. Removing the attribute would force us to
// either land all of Step 4+9 in one PR or sprinkle ad-hoc allows.
#[allow(dead_code)]
mod tracker;

#[cfg(not(any(target_os = "android", target_os = "ios")))]
mod tray;

#[cfg(not(any(target_os = "android", target_os = "ios")))]
use tauri::Manager;

#[cfg(not(any(target_os = "android", target_os = "ios")))]
use tauri_plugin_deep_link::DeepLinkExt;

use tauri_plugin_sql::{Builder as SqlBuilder, Migration, MigrationKind};

use crate::db::schema::initial_migration_sql;

// macOS code-signing & notarization reminder:
// `active-win-pos-rs` calls TCC-gated APIs (Accessibility / Screen
// Recording). macOS keys those grants by code-signing identity and bundle
// path. In `cargo tauri dev` the dev binary is ad-hoc-signed at the same
// path, so a one-time grant sticks. For released builds we MUST sign with a
// Developer ID and notarize, otherwise every rebuild is a fresh "untrusted"
// binary and the user has to re-grant Accessibility every time. See
// `tauri.conf.json > bundle.macOS` and `docs/release.md` (TBD).

/// Bring the main webview window to the foreground (used by `jadons-excuses://` URLs).
#[cfg(not(any(target_os = "android", target_os = "ios")))]
fn focus_main_window(app: &tauri::AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Migration v1: full 8-table schema + 6 categories + 140 app mappings.
    // See `db/schema.rs` for the rationale on full-reset.
    //
    // Migration v2: self-categorize the dev binary so it doesn't sit
    // forever in the "Uncategorized" bucket on the dashboard. We map
    // `'jadons-excuses'` (the cargo crate name + the macOS process name
    // in `cargo tauri dev`) to category 4 (Personal). `INSERT OR IGNORE`
    // makes this safe to re-run on a database that was hand-edited or
    // that has the row already (e.g. someone added it via INSERT OR
    // IGNORE at startup before this migration landed).
    let migrations = vec![
        Migration {
            version: 1,
            description: "create full schema and seed default categories + app mappings",
            sql: initial_migration_sql(),
            kind: MigrationKind::Up,
        },
        Migration {
            version: 2,
            description: "self-categorize jadons-excuses dev binary as Personal",
            sql: "INSERT OR IGNORE INTO app_category_map (pattern, pattern_type, category_id) \
                  VALUES ('jadons-excuses', 'app', 4);",
            kind: MigrationKind::Up,
        },
        Migration {
            version: 3,
            description: "insights tag column for dedupe",
            sql: "ALTER TABLE insights ADD COLUMN tag TEXT NOT NULL DEFAULT '';",
            kind: MigrationKind::Up,
        },
        Migration {
            version: 4,
            description: "index for browser bridge domain updates",
            sql: "CREATE INDEX IF NOT EXISTS idx_activity_browser \
                  ON activity_events(app_name, browser_domain, ended_at);",
            kind: MigrationKind::Up,
        },
        Migration {
            version: 5,
            description: "app_settings key-value store",
            sql: "CREATE TABLE IF NOT EXISTS app_settings ( \
                    key TEXT PRIMARY KEY NOT NULL, \
                    value TEXT NOT NULL \
                  );",
            kind: MigrationKind::Up,
        },
    ];

    let mut builder = tauri::Builder::default().plugin(
        SqlBuilder::default()
            .add_migrations("sqlite:jadons-excuses.db", migrations)
            .build(),
    );

    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    {
        builder = builder
            .plugin(tauri_plugin_notification::init())
            .plugin(
                tauri_plugin_global_shortcut::Builder::new()
                    .with_shortcuts(["CmdOrControl+Shift+J", "CmdOrControl+Shift+P"])
                    .expect("global shortcuts")
                    .with_handler(|app, shortcut, event| {
                        tray::on_global_shortcut(app, shortcut, event);
                    })
                    .build(),
            )
            .plugin(tauri_plugin_deep_link::init());
    }

    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    {
        builder = builder.on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                if window.label() == "main" {
                    let _ = window.hide();
                    api.prevent_close();
                }
            }
        });
    }

    let app = builder
        .setup(|app| {
            // Auto-start the foreground-window tracker. The setup closure
            // runs after `tauri-plugin-sql`'s plugin init, but before the
            // pool is necessarily ready — `spawn_tracker` handles that race
            // by waiting up to 10s for the pool to come online before its
            // first poll.
            crate::tracker::window::spawn_tracker(app.handle().clone());

            crate::tracker::browser_bridge::spawn_bridge(app.handle().clone());

            #[cfg(not(any(target_os = "android", target_os = "ios")))]
            {
                let h = app.handle().clone();
                let _ = app.deep_link().on_open_url(move |_| {
                    focus_main_window(&h);
                });
                if let Ok(Some(urls)) = app.deep_link().get_current() {
                    if !urls.is_empty() {
                        focus_main_window(app.handle());
                    }
                }

                let h2 = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    tokio::time::sleep(std::time::Duration::from_millis(250)).await;
                    if let Err(e) = tray::setup(&h2) {
                        eprintln!("[tray] deferred setup error: {e:?}");
                    }
                });
            }

            let h = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                use std::time::Duration;
                tokio::time::sleep(Duration::from_secs(3)).await;
                let _ = crate::analytics::summaries::rollup_stale_days(&h, 30).await;
                loop {
                    tokio::time::sleep(Duration::from_secs(3600)).await;
                    let _ = crate::analytics::summaries::rollup_stale_days(&h, 30).await;
                }
            });

            let h2 = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                use std::time::Duration;
                tokio::time::sleep(Duration::from_secs(10)).await;
                let _ = crate::analytics::insights::generate_insights(&h2).await;
                loop {
                    tokio::time::sleep(Duration::from_secs(1800)).await;
                    let _ = crate::analytics::insights::generate_insights(&h2).await;
                }
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::greet,
            commands::start_tracking,
            commands::stop_tracking,
            commands::db_health_check,
            commands::lookup_category_for_app,
            commands::list_categories,
            commands::list_app_mappings,
            commands::check_accessibility_permission,
            commands::request_accessibility_permission,
            commands::get_tracking_status,
            commands::get_recent_events,
            commands::check_input_monitoring_permission,
            commands::request_input_monitoring_permission,
            commands::get_current_engagement,
            commands::get_engagement_for_today,
            commands::get_engagement_for_event,
            commands::get_today_overview,
            commands::get_top_activity_today,
            commands::get_top_apps_today,
            commands::get_hourly_engagement_today,
            commands::get_activity_events,
            commands::get_activity_event_count,
            commands::get_aggregate_event_details,
            commands::get_setting,
            commands::set_setting,
            commands::send_test_notification,
            commands::pause_tracking_for,
            commands::recategorize_app,
            commands::get_trends_overview,
            commands::get_weekly_heatmap,
            commands::get_recent_insights,
            commands::generate_insights_now,
            commands::get_bridge_status,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application");

    app.run(|app_handle, event| {
        #[cfg(target_os = "macos")]
        if let tauri::RunEvent::Reopen { .. } = event {
            focus_main_window(app_handle);
        }
        #[cfg(not(target_os = "macos"))]
        drop(event);
    });
}
