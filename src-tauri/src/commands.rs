use serde::Serialize;
use tauri::{AppHandle, State};
use tauri_plugin_sql::DbInstances;

use chrono::Utc;

use crate::analytics::dashboard::{self, HourPoint, TodayOverview, TopActivity, TopApp};
use crate::analytics::insights;
use crate::analytics::trends;
use crate::db::queries::{
    self, ActivityEventWithCategory, ActivityFilters, AppMapping, Category, DbHealth,
};
use crate::tracker::browser_bridge;
use crate::tracker::engagement::{
    self, CurrentEngagement, EngagementSummary, EventEngagement,
};
use crate::tracker::window::{self, ActivityEvent, TrackingStatus};

#[derive(Debug, Serialize)]
pub struct CommandAck {
    pub ok: bool,
    pub message: String,
}

#[tauri::command]
pub fn greet(name: &str) -> String {
    format!("Hello, {name}! Welcome to Jadon's Excuses.")
}

#[tauri::command]
pub async fn stop_tracking(app: AppHandle) -> CommandAck {
    crate::tracker::window::pause_foreground_tracking(&app).await;
    CommandAck {
        ok: true,
        message: "tracking paused".into(),
    }
}

#[tauri::command]
pub fn start_tracking(app: AppHandle) -> CommandAck {
    crate::tracker::window::resume_foreground_tracking(app.clone());
    // The setup hook usually starts the tracker for us; this command exists
    // so the user can manually re-arm it after fixing permissions without
    // restarting the app (and it's idempotent — see `spawn_tracker`).
    window::spawn_tracker(app);
    CommandAck {
        ok: true,
        message: "tracking started".into(),
    }
}

// ---------------------------------------------------------------------------
// Tracker / permissions commands
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn check_accessibility_permission() -> bool {
    window::has_permissions()
}

#[tauri::command]
pub fn request_accessibility_permission() {
    // Opens System Settings → Privacy & Security → Accessibility on macOS.
    // The user still has to click the toggle and quit/relaunch — TCC grants
    // don't take effect until the next launch of the requesting binary.
    window::open_accessibility_pane();
}

#[tauri::command]
pub async fn get_tracking_status(app: AppHandle) -> Result<TrackingStatus, String> {
    Ok(window::get_tracking_status(&app).await)
}

#[tauri::command]
pub async fn get_recent_events(
    app: AppHandle,
    limit: u32,
) -> Result<Vec<ActivityEvent>, String> {
    window::get_recent_events(&app, limit).await
}

// ---------------------------------------------------------------------------
// Engagement commands (Step 4)
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn check_input_monitoring_permission() -> bool {
    engagement::has_input_permissions()
}

#[tauri::command]
pub fn request_input_monitoring_permission() {
    // macOS: opens System Settings → Privacy & Security → Input Monitoring.
    // The user toggles the app on, then must quit/relaunch — the TCC grant
    // doesn't apply to the running process.
    //
    // NOTE: With the device_query backend (post-Step 4 swap), the
    // operative TCC bucket on macOS is actually **Accessibility**, not
    // Input Monitoring — but Accessibility is already required by
    // active-win-pos-rs, so the user almost certainly has it granted.
    // We keep the legacy Input Monitoring pane open here as a safety
    // net: granting both buckets always works, granting neither never
    // works, and we don't want to retrain users mid-flight. On
    // Linux/Windows this is a no-op; device_query needs only X11 / no
    // special permission respectively.
    engagement::open_input_monitoring_pane();
}

#[tauri::command]
pub async fn get_current_engagement(app: AppHandle) -> CurrentEngagement {
    engagement::get_current_engagement(&app).await
}

#[tauri::command]
pub async fn get_engagement_for_today(app: AppHandle) -> Result<EngagementSummary, String> {
    engagement::get_engagement_for_today(&app).await
}

#[tauri::command]
pub async fn get_engagement_for_event(
    app: AppHandle,
    event_id: i64,
) -> Result<EventEngagement, String> {
    engagement::get_engagement_for_event(&app, event_id).await
}

// ---------------------------------------------------------------------------
// Dashboard commands (Step 6)
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn get_today_overview(app: AppHandle) -> Result<TodayOverview, String> {
    dashboard::get_today_overview(&app).await
}

#[tauri::command]
pub async fn get_top_activity_today(
    app: AppHandle,
    limit: u32,
) -> Result<Vec<TopActivity>, String> {
    dashboard::get_top_activity_today(&app, limit).await
}

#[tauri::command]
pub async fn get_top_apps_today(
    app: AppHandle,
    limit: u32,
) -> Result<Vec<TopApp>, String> {
    dashboard::get_top_apps_today(&app, limit).await
}

#[tauri::command]
pub async fn get_hourly_engagement_today(
    app: AppHandle,
) -> Result<Vec<HourPoint>, String> {
    dashboard::get_hourly_engagement_today(&app).await
}

// ---------------------------------------------------------------------------
// Activity timeline
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn get_activity_events(
    filters: ActivityFilters,
    instances: State<'_, DbInstances>,
) -> Result<Vec<ActivityEventWithCategory>, String> {
    let now_ms = Utc::now().timestamp_millis();
    queries::get_activity_events(&instances, filters, now_ms).await
}

#[tauri::command]
pub async fn get_activity_event_count(
    filters: ActivityFilters,
    instances: State<'_, DbInstances>,
) -> Result<u64, String> {
    let now_ms = Utc::now().timestamp_millis();
    queries::get_activity_event_count(&instances, filters, now_ms).await
}

#[tauri::command]
pub async fn get_aggregate_event_details(
    name: String,
    kind: String,
    filters: ActivityFilters,
    instances: State<'_, DbInstances>,
) -> Result<Vec<ActivityEventWithCategory>, String> {
    let now_ms = Utc::now().timestamp_millis();
    queries::get_aggregate_event_details(&instances, name, kind, filters, now_ms).await
}

#[tauri::command]
pub async fn get_setting(
    key: String,
    instances: State<'_, DbInstances>,
) -> Result<Option<String>, String> {
    queries::get_app_setting(&instances, &key).await
}

#[tauri::command]
pub async fn set_setting(
    key: String,
    value: String,
    instances: State<'_, DbInstances>,
) -> Result<(), String> {
    queries::set_app_setting(&instances, &key, &value).await
}

#[tauri::command]
pub fn send_test_notification(app: AppHandle) -> Result<(), String> {
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    {
        use tauri_plugin_notification::NotificationExt;
        app.notification()
            .builder()
            .title("Jadon's Excuses")
            .body("Notifications are working.")
            .show()
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
pub async fn pause_tracking_for(app: AppHandle, minutes: u32) -> CommandAck {
    use std::time::Duration;
    window::pause_foreground_tracking(&app).await;
    window::schedule_resume_foreground(app.clone(), Duration::from_secs(minutes as u64 * 60));
    CommandAck {
        ok: true,
        message: format!("paused for {} minutes", minutes),
    }
}

#[tauri::command]
pub async fn recategorize_app(
    app_name: String,
    category_id: i64,
    retroactive: bool,
    instances: State<'_, DbInstances>,
) -> Result<u32, String> {
    let n = queries::recategorize_app(&instances, app_name, category_id, retroactive).await?;
    window::clear_category_cache().await;
    Ok(n)
}

// ---------------------------------------------------------------------------
// Trends
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn get_trends_overview(
    app: AppHandle,
    days: u32,
) -> Result<trends::TrendsOverview, String> {
    trends::get_trends_overview(&app, days).await
}

#[tauri::command]
pub async fn get_weekly_heatmap(app: AppHandle) -> Result<Vec<trends::HeatmapCell>, String> {
    trends::get_weekly_heatmap(&app).await
}

// ---------------------------------------------------------------------------
// Insights
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn get_recent_insights(
    app: AppHandle,
    limit: u32,
) -> Result<Vec<insights::Insight>, String> {
    insights::get_recent_insights(&app, limit).await
}

#[tauri::command]
pub async fn generate_insights_now(app: AppHandle) -> Result<(), String> {
    insights::generate_insights(&app).await
}

#[tauri::command]
pub fn get_bridge_status() -> browser_bridge::BridgeStatus {
    browser_bridge::status()
}

// ---------------------------------------------------------------------------
// Database commands
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn db_health_check(
    app: AppHandle,
    instances: State<'_, DbInstances>,
) -> Result<DbHealth, String> {
    queries::db_health_check(&app, &instances).await
}

#[tauri::command]
pub async fn lookup_category_for_app(
    app_name: String,
    browser_domain: Option<String>,
    window_title: Option<String>,
    instances: State<'_, DbInstances>,
) -> Result<Option<Category>, String> {
    queries::lookup_category_for_app(app_name, browser_domain, window_title, &instances).await
}

#[tauri::command]
pub async fn list_categories(
    instances: State<'_, DbInstances>,
) -> Result<Vec<Category>, String> {
    queries::list_categories(&instances).await
}

#[tauri::command]
pub async fn list_app_mappings(
    category_id: Option<i64>,
    instances: State<'_, DbInstances>,
) -> Result<Vec<AppMapping>, String> {
    queries::list_app_mappings(category_id, &instances).await
}
