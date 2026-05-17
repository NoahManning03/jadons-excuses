//! Foreground-window tracking engine.
//!
//! This module owns the loop that turns the OS's active-window state into rows
//! in `activity_events` and `tab_switches`. A single Tokio task polls every
//! `POLL_INTERVAL` (1s by default) via the `active-win-pos-rs` crate and
//! diff-detects focus changes against the previous snapshot. On every change:
//!
//!   1. Close the previous open `activity_events` row (set `ended_at`,
//!      `duration_seconds`).
//!   2. Insert a `tab_switches` row recording the old → new app/window
//!      transition with the focus duration we just measured.
//!   3. Insert a fresh `activity_events` row for the new focus, with
//!      `ended_at = NULL`.
//!
//! Browser URL/domain are intentionally left `NULL` here — the browser
//! extension bridge (Step 9 in the roadmap) will fill them in later by
//! matching open events on `(app_name, started_at)`. We never block on the
//! bridge being available.
//!
//! ## macOS permissions
//!
//! `active-win-pos-rs` needs at least one of:
//!   - **Accessibility** permission for app/PID resolution via the
//!     `NSWorkspace` private bridge it falls back to in some configurations.
//!   - **Screen Recording** permission to populate window titles
//!     (otherwise `title` comes back as an empty string — see crate README).
//!
//! Both prompts are surfaced from the Privacy & Security pane in System
//! Settings. We open the Accessibility pane on user request because that's
//! the one that affects whether `get_active_window()` returns `Ok` or `Err`
//! at all in the common config — Screen Recording only affects title fidelity.
//!
//! On the first poll, if the call returns `Err(())` we treat that as a
//! permissions failure: flip `HAS_PERMISSIONS` to `false` and stop the loop.
//! The user can grant permission, quit, and relaunch — there's no clean way
//! to "rearm" a denied TCC prompt without an app restart on macOS anyway.
//!
//! ## Privacy / safety notes
//!
//! - We **never** store passwords, document bodies, clipboard contents, or
//!   any pixels from the screen. Only `app_name`, `window_title`, and
//!   `process_id` cross the boundary into SQLite.
//! - Window titles can leak filenames or document names. Users will be able
//!   to disable title capture from Settings in a later step; until then the
//!   capture is on by default because it's the single most useful signal
//!   for category resolution and "what was I doing at 3pm" recall.
//! - All data lives locally in SQLite under the app's config dir. Nothing
//!   is uploaded, ever. There is no telemetry endpoint.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use active_win_pos_rs::get_active_window;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use sqlx::{Pool, Row, Sqlite};
use tauri::{AppHandle, Manager};
use tauri_plugin_sql::{DbInstances, DbPool};
use tokio::sync::{Mutex, RwLock};

use crate::db::queries::DB_URL;

/// How often we sample the foreground window. Anything faster than ~500ms is
/// wasted CPU; anything slower than ~2s starts to miss the short Slack/Cmd-Tab
/// flickers that matter for the fragmentation score.
const POLL_INTERVAL: Duration = Duration::from_millis(1000);

/// Browser app names we recognize across macOS, Windows, and Linux. The
/// extension bridge (Step 9) will key off this list to know which app's
/// activity_event rows to back-fill with `browser_url`/`browser_domain`.
pub const BROWSER_APPS: &[&str] = &[
    "Google Chrome",
    "Chrome",
    "Microsoft Edge",
    "Edge",
    "Firefox",
    "Safari",
    "Brave Browser",
    "Brave",
    "Arc",
    "Vivaldi",
    "Opera",
];

/// True once the first `get_active_window()` call has succeeded. Stays
/// `false` if the very first call fails — that's our heuristic for "TCC
/// denied us". See module-level docs for the macOS rationale.
static HAS_PERMISSIONS: AtomicBool = AtomicBool::new(false);

/// Set once the polling task has actually started (so the UI can distinguish
/// "tracker disabled" from "tracker waiting on permissions").
static TRACKING_RUNNING: AtomicBool = AtomicBool::new(false);

/// Best-effort UTC midnight cutoff for "today" (ms since epoch). Recomputed
/// each time we count, so day rollovers are fine.
static TOTAL_EVENTS_TODAY: AtomicU64 = AtomicU64::new(0);

/// When true, the foreground tracker loop sleeps without polling the OS or
/// writing rows — used by the tray "pause tracking" flows.
static FOREGROUND_PAUSED: AtomicBool = AtomicBool::new(false);

/// Incremented whenever we pause or resume so the in-loop `prev` / open-event
/// state resets cleanly after an external `UPDATE … ended_at` closeout.
static TRACKER_EPOCH: AtomicU64 = AtomicU64::new(0);

/// Snapshot of the currently focused window, shared via `Arc<RwLock<>>` so
/// other modules (engagement.rs in Step 4) can sample it without going
/// through the DB.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ForegroundWindow {
    pub app_name: String,
    pub window_title: Option<String>,
    pub pid: Option<u32>,
}

/// One row of `activity_events`, suitable for serializing to the frontend.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActivityEvent {
    pub id: i64,
    pub app_name: String,
    pub window_title: Option<String>,
    pub browser_url: Option<String>,
    pub browser_domain: Option<String>,
    pub category_id: Option<i64>,
    pub started_at: i64,
    pub ended_at: Option<i64>,
    pub duration_seconds: Option<i64>,
}

/// Status snapshot consumed by `commands::get_tracking_status` and rendered
/// in the Settings → DB Health panel.
#[derive(Debug, Clone, Serialize)]
pub struct TrackingStatus {
    pub running: bool,
    pub current_app: Option<String>,
    pub current_window_title: Option<String>,
    pub has_permissions: bool,
    pub total_events_today: u64,
}

// --- shared state ---------------------------------------------------------

/// Read by `current()` for any caller that wants the most recent foreground
/// snapshot without polling itself. `RwLock` (vs `Mutex`) because the read
/// path will eventually be hit at engagement-sample frequency (every ~5s)
/// while the write path is once per second; readers should never serialize.
fn current_window_handle() -> &'static Arc<RwLock<Option<ForegroundWindow>>> {
    use std::sync::OnceLock;
    static H: OnceLock<Arc<RwLock<Option<ForegroundWindow>>>> = OnceLock::new();
    H.get_or_init(|| Arc::new(RwLock::new(None)))
}

/// Cached app_name → category_id lookups. We rebuild this on every tracker
/// start (and will rebuild it on category-edit signals once Settings supports
/// editing). Worth keeping because we'd otherwise round-trip the DB once per
/// focus change, and focus changes spike during Cmd-Tab storms.
fn category_cache_handle() -> &'static Arc<Mutex<HashMap<String, Option<i64>>>> {
    use std::sync::OnceLock;
    static H: OnceLock<Arc<Mutex<HashMap<String, Option<i64>>>>> = OnceLock::new();
    H.get_or_init(|| Arc::new(Mutex::new(HashMap::new())))
}

/// Clear the in-memory app → category cache so the next resolve hits SQLite.
/// Call after mutating `app_category_map` (e.g. Settings → recategorize).
pub async fn clear_category_cache() {
    let mut cache = category_cache_handle().lock().await;
    cache.clear();
}

// --- public API -----------------------------------------------------------

/// Last-known foreground snapshot, or `None` if we haven't polled yet (or
/// the user denied permission and we never started).
pub async fn current() -> Option<ForegroundWindow> {
    current_window_handle().read().await.clone()
}

pub fn has_permissions() -> bool {
    HAS_PERMISSIONS.load(Ordering::SeqCst)
}

pub fn is_running() -> bool {
    TRACKING_RUNNING.load(Ordering::SeqCst)
}

/// True while tray / shortcuts have paused foreground + engagement capture.
pub fn is_foreground_paused() -> bool {
    FOREGROUND_PAUSED.load(Ordering::SeqCst)
}

pub fn total_events_today() -> u64 {
    TOTAL_EVENTS_TODAY.load(Ordering::SeqCst)
}

/// Close every still-open `activity_events` row with `ended_at = now`.
pub async fn close_open_activity_events(app: &AppHandle) -> Result<(), String> {
    let pool = sqlite_pool(app).await?;
    let now = now_ms();
    sqlx::query(
        "UPDATE activity_events \
         SET ended_at = ?, \
             duration_seconds = CASE \
               WHEN ? >= started_at THEN (? - started_at) / 1000 \
               ELSE 0 \
             END \
         WHERE ended_at IS NULL",
    )
    .bind(now)
    .bind(now)
    .bind(now)
    .execute(&pool)
    .await
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// Pause window + engagement tracking. Idempotent.
pub async fn pause_foreground_tracking(app: &AppHandle) {
    if FOREGROUND_PAUSED.swap(true, Ordering::SeqCst) {
        return;
    }
    let _ = close_open_activity_events(app).await;
    crate::tracker::engagement::stop();
    TRACKER_EPOCH.fetch_add(1, Ordering::SeqCst);
}

/// Resume after a tray / shortcut pause. Re-arms engagement sampling.
pub fn resume_foreground_tracking(app: AppHandle) {
    if !FOREGROUND_PAUSED.swap(false, Ordering::SeqCst) {
        return;
    }
    TRACKER_EPOCH.fetch_add(1, Ordering::SeqCst);
    if has_permissions() {
        crate::tracker::engagement::start(app);
    }
}

/// Schedule [`resume_foreground_tracking`] on a timer (15 / 30 / 60 minutes).
pub fn schedule_resume_foreground(app: AppHandle, after: std::time::Duration) {
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(after).await;
        resume_foreground_tracking(app);
    });
}

/// Spawn the polling loop on the current Tokio runtime. Idempotent — calling
/// it twice is a no-op; we use `compare_exchange` on `TRACKING_RUNNING` to
/// prevent duplicate loops if both `start_tracking()` (manual) and the setup
/// hook fire it.
pub fn spawn_tracker(app: AppHandle) {
    if TRACKING_RUNNING
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return; // already running
    }

    // Sibling module needs to know we've started (mainly for the UI flag and
    // for engagement sampling to gate on whether there's an active event id).
    // The engagement engine wants its own AppHandle clone so it can resolve
    // the SQLite pool independently of this loop.
    //
    // Do not call `engagement::start` until `get_active_window()` has
    // succeeded at least once (`has_permissions()`). On macOS 26.x,
    // `device_query::DeviceState::new()` can abort() while Accessibility is
    // still transitional at launch; `catch_unwind` cannot catch that.
    let app_for_engagement = app.clone();
    tauri::async_runtime::spawn(async move {
        loop {
            if has_permissions() {
                crate::tracker::engagement::start(app_for_engagement);
                break;
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
    });

    tauri::async_runtime::spawn(async move {
        run_tracker_loop(app).await;
        TRACKING_RUNNING.store(false, Ordering::SeqCst);
        crate::tracker::engagement::stop();
    });
}

// --- internal -------------------------------------------------------------

async fn sqlite_pool(app: &AppHandle) -> Result<Pool<Sqlite>, String> {
    let instances = app.state::<DbInstances>();
    let map = instances.0.read().await;
    let pool = map
        .get(DB_URL)
        .ok_or_else(|| format!("database '{DB_URL}' not loaded"))?;
    match pool {
        DbPool::Sqlite(p) => Ok(p.clone()),
        #[allow(unreachable_patterns)]
        _ => Err("expected sqlite pool".into()),
    }
}

/// Wait for `tauri-plugin-sql` to finish opening the connection pool. The
/// plugin loads asynchronously during setup; if the tracker spawns first we
/// might race it. Cap retries so a permanently-broken DB doesn't deadlock.
async fn wait_for_pool(app: &AppHandle) -> Option<Pool<Sqlite>> {
    for _ in 0..40 {
        if let Ok(pool) = sqlite_pool(app).await {
            return Some(pool);
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    eprintln!("[tracker] gave up waiting for DB pool after 10s");
    None
}

async fn refresh_category_cache(pool: &Pool<Sqlite>) {
    // Pull every (app, category_id) pair into RAM in one go. There are
    // ~140 mappings seeded; even if a power user 10x's that, this is a
    // single-digit-KB hash table.
    let rows = sqlx::query(
        "SELECT pattern, category_id FROM app_category_map WHERE pattern_type = 'app'",
    )
    .fetch_all(pool)
    .await;

    let mut cache = category_cache_handle().lock().await;
    cache.clear();
    if let Ok(rows) = rows {
        for r in rows {
            let pattern: String = r.get("pattern");
            let cid: i64 = r.get("category_id");
            cache.insert(pattern, Some(cid));
        }
    }
}

/// Resolve a category_id for the given app, hitting the cache first and
/// falling back to a DB lookup that also caches the negative result so we
/// don't re-query for unknown apps every focus change.
async fn resolve_category_id(pool: &Pool<Sqlite>, app_name: &str) -> Option<i64> {
    {
        let cache = category_cache_handle().lock().await;
        if let Some(hit) = cache.get(app_name) {
            return *hit;
        }
    }

    // Cache miss — look it up. We only check the 'app' pattern type here on
    // purpose; domain/title resolution happens later when the browser bridge
    // fills in URL+domain (it'll re-categorize via lookup_category_for_app).
    let row = sqlx::query(
        "SELECT category_id FROM app_category_map \
         WHERE pattern_type = 'app' AND pattern = ? LIMIT 1",
    )
    .bind(app_name)
    .fetch_optional(pool)
    .await
    .ok()
    .flatten();
    let cid: Option<i64> = row.map(|r| r.get("category_id"));

    let mut cache = category_cache_handle().lock().await;
    cache.insert(app_name.to_string(), cid);
    cid
}

/// Refresh `TOTAL_EVENTS_TODAY` from the DB. Called once per focus change
/// (cheap — single COUNT on an indexed column).
async fn refresh_today_count(pool: &Pool<Sqlite>) {
    let start_of_today = start_of_today_utc_ms();
    let res: Result<(i64,), _> = sqlx::query_as(
        "SELECT COUNT(*) FROM activity_events WHERE started_at >= ?",
    )
    .bind(start_of_today)
    .fetch_one(pool)
    .await;
    if let Ok((c,)) = res {
        TOTAL_EVENTS_TODAY.store(c as u64, Ordering::SeqCst);
    }
}

fn start_of_today_utc_ms() -> i64 {
    let now = Utc::now();
    let midnight = now
        .date_naive()
        .and_hms_opt(0, 0, 0)
        .unwrap()
        .and_utc();
    midnight.timestamp_millis()
}

fn now_ms() -> i64 {
    Utc::now().timestamp_millis()
}

/// True if `a` and `b` describe the same focus (same app + same title).
/// `None` window titles compare equal to other `None`s — that means an app
/// with a stripped/empty title won't generate spurious focus changes.
fn same_focus(a: &ForegroundWindow, b: &ForegroundWindow) -> bool {
    a.app_name == b.app_name && a.window_title == b.window_title
}

/// Convert the crate's `ActiveWindow` into our internal struct. Empty title
/// → `None` so the DB stores NULL rather than ""; that keeps category lookups
/// (which use `LIKE '%' || pattern || '%'`) from matching every empty title.
fn from_active_window(w: active_win_pos_rs::ActiveWindow) -> ForegroundWindow {
    let title = if w.title.trim().is_empty() {
        None
    } else {
        Some(w.title)
    };
    ForegroundWindow {
        app_name: w.app_name,
        window_title: title,
        // active-win-pos-rs uses u64 for process_id; downcast is safe — no
        // OS we target hands out PIDs above 2^32.
        pid: Some(w.process_id as u32),
    }
}

/// Close a single open activity row by id. Reads `started_at` from the DB so
/// callers stay correct after the browser bridge splits a Chrome session into
/// multiple rows. Skips rows already closed (`ended_at` set).
pub(crate) async fn close_activity_event_if_open(
    pool: &Pool<Sqlite>,
    event_id: i64,
    ended_at: i64,
) -> Result<(), String> {
    let row = sqlx::query(
        "SELECT started_at, ended_at FROM activity_events WHERE id = ?",
    )
    .bind(event_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| e.to_string())?;
    let Some(row) = row else {
        return Ok(());
    };
    if row.get::<Option<i64>, _>("ended_at").is_some() {
        return Ok(());
    }
    let started_at: i64 = row.get("started_at");
    let duration_seconds = (ended_at - started_at).max(0) / 1000;
    let res = sqlx::query(
        "UPDATE activity_events SET ended_at = ?, duration_seconds = ? \
         WHERE id = ? AND ended_at IS NULL",
    )
    .bind(ended_at)
    .bind(duration_seconds)
    .bind(event_id)
    .execute(pool)
    .await;
    match res {
        Ok(_) => Ok(()),
        Err(e) => {
            eprintln!("[tracker] failed to close event {event_id}: {e}");
            Err(e.to_string())
        }
    }
}

async fn fetch_event_started_at(pool: &Pool<Sqlite>, event_id: i64) -> Option<i64> {
    sqlx::query_scalar::<_, i64>("SELECT started_at FROM activity_events WHERE id = ?")
        .bind(event_id)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten()
}

async fn close_event(pool: &Pool<Sqlite>, event_id: i64, ended_at: i64) {
    if let Err(e) = close_activity_event_if_open(pool, event_id, ended_at).await {
        eprintln!("[tracker] close_event: {e}");
    }
}

async fn insert_tab_switch(
    pool: &Pool<Sqlite>,
    from_app: Option<&str>,
    to_app: &str,
    switched_at: i64,
    focus_duration_seconds: i64,
) {
    let res = sqlx::query(
        "INSERT INTO tab_switches \
            (from_app, to_app, from_url, to_url, switched_at, focus_duration_seconds) \
         VALUES (?, ?, NULL, NULL, ?, ?)",
    )
    .bind(from_app)
    .bind(to_app)
    .bind(switched_at)
    .bind(focus_duration_seconds)
    .execute(pool)
    .await;
    if let Err(e) = res {
        eprintln!("[tracker] failed to insert tab_switch: {e}");
    }
}

async fn insert_activity_event(
    pool: &Pool<Sqlite>,
    win: &ForegroundWindow,
    category_id: Option<i64>,
    started_at: i64,
) -> Option<i64> {
    // browser_url / browser_domain are intentionally NULL on insert. The
    // browser extension (Step 9) backfills them by matching the most recent
    // open event for the matching browser app.
    let res = sqlx::query(
        "INSERT INTO activity_events \
            (app_name, window_title, browser_url, browser_domain, \
             category_id, started_at, ended_at, duration_seconds) \
         VALUES (?, ?, NULL, NULL, ?, ?, NULL, NULL)",
    )
    .bind(&win.app_name)
    .bind(win.window_title.as_deref())
    .bind(category_id)
    .bind(started_at)
    .execute(pool)
    .await;

    match res {
        Ok(r) => Some(r.last_insert_rowid()),
        Err(e) => {
            eprintln!("[tracker] failed to insert activity_event: {e}");
            None
        }
    }
}

/// Best-effort cleanup of any `activity_events` rows that were left with
/// `ended_at = NULL` because a previous app session crashed or was
/// force-quit while focused on a window.
///
/// Strategy:
///   * "Stale" = `ended_at IS NULL` AND `started_at < now - 5 minutes`.
///     The 5-minute cutoff means a *currently* open event from this
///     session (started seconds ago) is never touched — only ghosts
///     from prior sessions.
///   * Each ghost is closed with a 30-second default duration. We can't
///     know the real value (the user could have stayed focused for ten
///     minutes or for one second), so 30s is a deliberately neutral
///     "they at least glanced at it" estimate that won't badly skew
///     daily totals.
///
/// This runs once on tracker startup and is idempotent: a second call
/// finds nothing to update because the `WHERE ended_at IS NULL` clause
/// no longer matches.
async fn reconcile_stale_open_events(pool: &Pool<Sqlite>) {
    let cutoff = now_ms() - 5 * 60 * 1000;
    // We compute ended_at = started_at + 30000 in SQL so the row's
    // duration_seconds and ended_at stay self-consistent.
    let res = sqlx::query(
        "UPDATE activity_events \
            SET ended_at = started_at + 30000, \
                duration_seconds = 30 \
          WHERE ended_at IS NULL AND started_at < ?",
    )
    .bind(cutoff)
    .execute(pool)
    .await;
    match res {
        Ok(r) if r.rows_affected() > 0 => {
            eprintln!(
                "[tracker] reconciled {} stale open activity_events from prior sessions",
                r.rows_affected()
            );
        }
        Ok(_) => {} // nothing to do
        Err(e) => eprintln!("[tracker] failed to reconcile stale events: {e}"),
    }
}

async fn run_tracker_loop(app: AppHandle) {
    let Some(pool) = wait_for_pool(&app).await else {
        return;
    };

    reconcile_stale_open_events(&pool).await;
    refresh_category_cache(&pool).await;
    refresh_today_count(&pool).await;

    let mut prev: Option<ForegroundWindow> = None;
    let mut first_poll = true;
    let mut local_epoch = TRACKER_EPOCH.load(Ordering::Relaxed);

    loop {
        // tokio::time::interval would tick on a fixed clock, which means a
        // slow poll cycle would coalesce ticks and we'd burn CPU catching
        // up. A plain sleep gives us "at least 1s between observations",
        // which is what we actually want.
        tokio::time::sleep(POLL_INTERVAL).await;

        let gen = TRACKER_EPOCH.load(Ordering::Relaxed);
        if gen != local_epoch {
            prev = None;
            local_epoch = gen;
        }

        if FOREGROUND_PAUSED.load(Ordering::Relaxed) {
            continue;
        }

        let next = match get_active_window() {
            Ok(w) => from_active_window(w),
            Err(()) => {
                if first_poll {
                    // Treat an immediate hard failure as TCC denial. Don't
                    // crash, don't keep spinning — wait for the user to
                    // grant + relaunch. The Settings panel polls
                    // `get_tracking_status()` every 2s and will reflect
                    // `has_permissions = false` until then.
                    HAS_PERMISSIONS.store(false, Ordering::SeqCst);
                    eprintln!(
                        "[tracker] active-win-pos-rs returned Err on first poll — \
                         likely missing macOS Accessibility/Screen Recording permission. \
                         Stopping tracker until next launch."
                    );
                    return;
                }
                // Transient failure mid-run (app unfocused, fullscreen flip,
                // Mission Control open, etc.). Just skip this tick.
                continue;
            }
        };

        // First successful poll → permissions confirmed.
        if first_poll {
            HAS_PERMISSIONS.store(true, Ordering::SeqCst);
            first_poll = false;
        }

        // Update shared snapshot regardless of focus-change outcome.
        *current_window_handle().write().await = Some(next.clone());

        let is_change = match prev.as_ref() {
            None => true,
            Some(p) => !same_focus(p, &next),
        };
        if !is_change {
            continue;
        }

        let now = now_ms();

        let prev_eid = crate::tracker::engagement::current_event_id();
        let focus_duration = if let Some(eid) = prev_eid {
            let focus_start = fetch_event_started_at(&pool, eid).await.unwrap_or(now);
            (now - focus_start).max(0) / 1000
        } else {
            0
        };

        // 1. Close the previous open event (no-op if browser_bridge already closed it).
        if let Some(eid) = prev_eid {
            close_event(&pool, eid, now).await;
        }

        // 2. Record the transition.
        let from_app = prev.as_ref().map(|p| p.app_name.as_str());
        insert_tab_switch(&pool, from_app, &next.app_name, now, focus_duration).await;

        // 3. Open a new event.
        let cid = resolve_category_id(&pool, &next.app_name).await;
        let new_id = insert_activity_event(&pool, &next, cid, now).await;
        crate::tracker::engagement::set_current_event_id(new_id);

        prev = Some(next);
        refresh_today_count(&pool).await;
    }
}

// --- query helpers exposed via Tauri commands -----------------------------

/// Recent rows from `activity_events`, newest first. Backs the
/// `get_recent_events` Tauri command for the dashboard.
pub async fn get_recent_events(
    app: &AppHandle,
    limit: u32,
) -> Result<Vec<ActivityEvent>, String> {
    let pool = sqlite_pool(app).await?;
    // Cap to a sane upper bound so a runaway frontend can't drag a
    // multi-million-row table over the IPC channel.
    let limit = limit.clamp(1, 1000) as i64;
    let rows = sqlx::query(
        "SELECT id, app_name, window_title, browser_url, browser_domain, \
                category_id, started_at, ended_at, duration_seconds \
         FROM activity_events \
         ORDER BY started_at DESC \
         LIMIT ?",
    )
    .bind(limit)
    .fetch_all(&pool)
    .await
    .map_err(|e| e.to_string())?;

    Ok(rows
        .iter()
        .map(|r| ActivityEvent {
            id: r.get("id"),
            app_name: r.get("app_name"),
            window_title: r.get("window_title"),
            browser_url: r.get("browser_url"),
            browser_domain: r.get("browser_domain"),
            category_id: r.get("category_id"),
            started_at: r.get("started_at"),
            ended_at: r.get("ended_at"),
            duration_seconds: r.get("duration_seconds"),
        })
        .collect())
}

/// Compose the snapshot the Settings panel polls every 2s.
pub async fn get_tracking_status(app: &AppHandle) -> TrackingStatus {
    let snap = current().await;
    // Refresh today's count opportunistically. Cheap query, and avoids
    // showing a stale value if the user has been idle on the Settings page.
    if let Ok(pool) = sqlite_pool(app).await {
        refresh_today_count(&pool).await;
    }
    TrackingStatus {
        running: is_running(),
        current_app: snap.as_ref().map(|s| s.app_name.clone()),
        current_window_title: snap.as_ref().and_then(|s| s.window_title.clone()),
        has_permissions: has_permissions(),
        total_events_today: total_events_today(),
    }
}

/// Open System Settings → Privacy & Security → Accessibility on macOS. On
/// other platforms this is a no-op (we expect the caller to gate the button).
pub fn open_accessibility_pane() {
    #[cfg(target_os = "macos")]
    {
        // The classic `x-apple.systempreferences:` URL still works on
        // Sonoma/Sequoia; macOS routes legacy "preferences" panes to the
        // new System Settings app automatically.
        let _ = std::process::Command::new("open")
            .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility")
            .spawn();
    }
    #[cfg(not(target_os = "macos"))]
    {
        eprintln!("[tracker] open_accessibility_pane is a no-op outside macOS");
    }
}

// NOTE on macOS code-signing & notarization:
// macOS's TCC database keys Accessibility / Screen Recording grants by the
// app's code-signing identity (or, in the unsigned case, by the ad-hoc
// signature + bundle path). Every `cargo tauri build` re-signs with a fresh
// ad-hoc identity, which can cause macOS to forget the previous grant. For
// production we'll sign with a Developer ID and notarize so the grant
// persists across upgrades. See `tauri.conf.json > bundle.macOS`.
