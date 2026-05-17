//! Dashboard aggregations.
//!
//! The Dashboard page is a one-shot read API: every 15s the frontend pulls
//! three small JSON blobs (`TodayOverview`, top apps, hourly engagement)
//! and re-paints. We keep all of that math here so the queries are
//! testable in isolation and the Tauri command layer stays a thin shim.
//!
//! Performance notes:
//!   * Every query is scoped to "today" (UTC midnight → now). The
//!     activity_events index on `started_at` and the engagement_samples
//!     index on `sampled_at` make all three queries linear in today's
//!     row count, which never exceeds ~10k samples on a heavy day.
//!   * `LEFT JOIN app_category_map` picks up freshly-seeded mappings
//!     (e.g. v2's `jadons-excuses → Personal`) even for events that were
//!     inserted with `category_id = NULL` before the seed landed. This
//!     means historic data "self-heals" the moment a new mapping is
//!     added — no row rewrites required.
//!   * Composite focus_score formula is documented on `compute_focus_score`
//!     below; that doc is the source of truth referenced by Insights.tsx.

use chrono::Utc;
use serde::{Deserialize, Serialize};
use sqlx::sqlite::SqliteRow;
use sqlx::{Pool, Row, Sqlite};
use tauri::{AppHandle, Manager};
use tauri_plugin_sql::{DbInstances, DbPool};

use crate::analytics::fragmentation;
use crate::db::queries::DB_URL;
use crate::tracker::window::BROWSER_APPS;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TodayOverview {
    /// Wall-clock seconds we've actually been tracking today. Closes out
    /// the currently-open event with `(now - started_at)` so the value
    /// doesn't visibly jump every focus change.
    pub tracked_seconds: u64,
    /// Seconds spent with engagement_score > 0. Each engagement_samples
    /// row represents 10s of clock time.
    pub active_seconds: u64,
    /// Seconds spent with engagement_score = 0 (idle).
    pub idle_seconds: u64,
    /// Seconds spent with engagement_score >= 81 (the "intense" bucket).
    /// Useful for "deep work" tallies.
    pub intense_seconds: u64,
    /// Distinct app/window switches today.
    pub switch_count: u32,
    /// 0 (calm) → 100 (chaotic). See [`fragmentation::score`].
    pub fragmentation_score: u8,
    /// Composite 0–100 quality score. See [`compute_focus_score`].
    pub focus_score: u8,
    /// Rows in `engagement_samples` since UTC midnight (same filter as averages).
    /// When zero, `focus_score` is defined as 0 — UI may show "not enough data".
    pub engagement_sample_count: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TopApp {
    pub app_name: String,
    pub total_seconds: i64,
    pub event_count: u32,
    pub category_id: Option<i64>,
    pub category_name: Option<String>,
    pub category_color: Option<String>,
    /// Seconds within this app where engagement_score > 0. Always
    /// ≤ `total_seconds` modulo a sub-bucket rounding error.
    pub active_seconds: u64,
    pub avg_engagement: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HourPoint {
    /// 0..=23, UTC-day bucket.
    pub hour: u8,
    pub avg_engagement: u8,
    /// 0..=60 — number of distinct calendar minutes within this hour
    /// that contained at least one non-idle (`engagement_score > 0`)
    /// sample. Capped at 60 so the chart axis is always sane.
    pub active_minutes: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TopActivity {
    /// `"app"` or `"website"` for the frontend.
    pub kind: String,
    pub name: String,
    pub icon_hint: Option<String>,
    pub total_seconds: i64,
    pub active_seconds: i64,
    pub avg_engagement: u8,
    pub category_name: Option<String>,
    pub category_color: Option<String>,
}

// --- helpers -------------------------------------------------------------

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

fn start_of_today_utc_ms() -> i64 {
    let now = Utc::now();
    let midnight = match now.date_naive().and_hms_opt(0, 0, 0) {
        Some(dt) => dt.and_utc(),
        None => {
            eprintln!(
                "[dashboard] start_of_today: and_hms_opt(0,0,0) failed — using current instant"
            );
            now
        }
    };
    midnight.timestamp_millis()
}

fn now_ms() -> i64 {
    Utc::now().timestamp_millis()
}

/// Seconds in one engagement sample. Mirrors `SAMPLE_INTERVAL_MS / 1000`
/// in `tracker::engagement` — kept as a literal here on purpose so a
/// future change there doesn't silently rescale historical samples.
const SECS_PER_SAMPLE: u64 = 10;

// --- safe row decoding (never panic the tokio worker on type mismatch) ---

/// SQLite `AVG()` is `REAL`. `COALESCE(avg, 0)` without a float literal can
/// yield `INTEGER` for the NULL branch — sqlx then rejects `Option<f64>`.
fn decode_avg_score_u8(row: &SqliteRow, col: &str) -> u8 {
    let v: Option<f64> = row
        .try_get::<Option<f64>, _>(col)
        .or_else(|_| row.try_get::<f64, _>(col).map(Some))
        .or_else(|_| {
            row.try_get::<Option<i64>, _>(col)
                .map(|o| o.map(|i| i as f64))
        })
        .or_else(|_| row.try_get::<i64, _>(col).map(|i| Some(i as f64)))
        .unwrap_or_else(|e| {
            eprintln!("[dashboard] decode {col} as avg score: {e}");
            None
        });
    v.unwrap_or(0.0)
        .round()
        .clamp(0.0, 100.0) as u8
}

fn decode_i64(row: &SqliteRow, col: &str, default: i64) -> i64 {
    row.try_get::<i64, _>(col)
        .or_else(|_| {
            row.try_get::<Option<i64>, _>(col)
                .map(|o| o.unwrap_or(default))
        })
        .unwrap_or_else(|e| {
            eprintln!("[dashboard] decode {col} as i64: {e}");
            default
        })
}

fn decode_opt_i64_sum(row: &SqliteRow, col: &str) -> Option<i64> {
    row.try_get::<Option<i64>, _>(col)
        .or_else(|_| row.try_get::<i64, _>(col).map(Some))
        .unwrap_or_else(|e| {
            eprintln!("[dashboard] decode {col} as Option<i64> (SUM): {e}");
            None
        })
}

fn decode_string(row: &SqliteRow, col: &str, default: &str) -> String {
    row.try_get::<String, _>(col).unwrap_or_else(|e| {
        eprintln!("[dashboard] decode {col} as String: {e}");
        default.to_string()
    })
}

fn decode_opt_i64(row: &SqliteRow, col: &str) -> Option<i64> {
    row.try_get::<Option<i64>, _>(col).unwrap_or_else(|e| {
        eprintln!("[dashboard] decode {col} as Option<i64>: {e}");
        None
    })
}

fn decode_opt_string(row: &SqliteRow, col: &str) -> Option<String> {
    row.try_get::<Option<String>, _>(col).unwrap_or_else(|e| {
        eprintln!("[dashboard] decode {col} as Option<String>: {e}");
        None
    })
}

// --- focus score ---------------------------------------------------------

/// Composite quality score for the day, 0–100.
///
/// ```text
///   focus_score = 0.50 * engagement_avg
///               + 0.30 * (100 - fragmentation_score)
///               + 0.20 * focus_pct
/// ```
///
/// Where:
///   * `engagement_avg` is the average `engagement_score` across all of
///     today's `engagement_samples`. Reading this signal is half the
///     score because *being engaged* is the strongest "real work
///     happening" indicator we have.
///   * `(100 - fragmentation_score)` rewards continuity: if you only
///     switched apps a couple of times today you keep all 30 points; if
///     you bounced every 10s you keep ~0.
///   * `focus_pct` is the percentage of tracked seconds spent inside
///     categories whose `productivity_level` is `'focus'` or `'work'`.
///     This is a fairly soft signal because category seeding is
///     imperfect (browsers default to NULL until the bridge backfills)
///     so we cap its weight at 20%.
///
/// **Hard zeros.** If we have no engagement samples at all, *or* no
/// tracked time at all, we return 0 instead of a fragmentation-only
/// score. Rationale: at zero data, "no fragmentation because there's
/// nothing to fragment" would surface as ~30, which is misleading.
pub fn compute_focus_score(
    engagement_avg: u8,
    fragmentation_score: u8,
    focus_pct: u8,
    total_samples: u64,
    tracked_seconds: u64,
) -> u8 {
    if total_samples == 0 || tracked_seconds == 0 {
        return 0;
    }
    let s = 0.50 * engagement_avg as f32
        + 0.30 * (100.0 - fragmentation_score as f32)
        + 0.20 * focus_pct as f32;
    s.clamp(0.0, 100.0).round() as u8
}

// --- queries -------------------------------------------------------------

pub async fn get_today_overview(app: &AppHandle) -> Result<TodayOverview, String> {
    let pool = sqlite_pool(app).await?;
    let start = start_of_today_utc_ms();
    let now = now_ms();

    // 1) tracked_seconds: SUM duration_seconds, treating NULL ended_at
    //    as "still open right now". Done in SQL so we're not pulling
    //    every event row across the IPC boundary.
    let tracked_row = sqlx::query(
        "SELECT COALESCE(SUM( \
                COALESCE(duration_seconds, (? - started_at) / 1000) \
            ), 0) AS s \
         FROM activity_events \
         WHERE started_at >= ?",
    )
    .bind(now)
    .bind(start)
    .fetch_one(&pool)
    .await
    .map_err(|e| e.to_string())?;
    let tracked_seconds = decode_i64(&tracked_row, "s", 0).max(0) as u64;

    // 2) engagement bucket counts.
    let buckets = sqlx::query(
        "SELECT \
           SUM(CASE WHEN engagement_score = 0 THEN 1 ELSE 0 END) AS idle_c, \
           SUM(CASE WHEN engagement_score > 0 THEN 1 ELSE 0 END) AS active_c, \
           SUM(CASE WHEN engagement_score >= 81 THEN 1 ELSE 0 END) AS intense_c, \
           CAST(AVG(engagement_score) AS REAL) AS avg_score, \
           COUNT(*) AS total_c \
         FROM engagement_samples \
         WHERE sampled_at >= ?",
    )
    .bind(start)
    .fetch_one(&pool)
    .await
    .map_err(|e| e.to_string())?;

    let idle_c = decode_opt_i64_sum(&buckets, "idle_c");
    let active_c = decode_opt_i64_sum(&buckets, "active_c");
    let intense_c = decode_opt_i64_sum(&buckets, "intense_c");
    let total_c = decode_i64(&buckets, "total_c", 0);
    let engagement_avg = decode_avg_score_u8(&buckets, "avg_score");

    let to_secs = |c: Option<i64>| -> u64 {
        (c.unwrap_or(0).max(0) as u64).saturating_mul(SECS_PER_SAMPLE)
    };
    let active_seconds = to_secs(active_c);
    let idle_seconds = to_secs(idle_c);
    let intense_seconds = to_secs(intense_c);
    let total_samples = total_c.max(0) as u64;

    // 3) switch_count.
    let switch_row = sqlx::query(
        "SELECT COUNT(*) AS c FROM tab_switches WHERE switched_at >= ?",
    )
    .bind(start)
    .fetch_one(&pool)
    .await
    .map_err(|e| e.to_string())?;
    let switch_count_i64 = decode_i64(&switch_row, "c", 0);
    let switch_count: u32 = switch_count_i64.clamp(0, u32::MAX as i64) as u32;

    // 4) fragmentation: switches per minute over observed seconds,
    //    normalized 0..1, then × 100. We use `tracked_seconds` as the
    //    denominator because it's the cleanest "we had focus" signal.
    let frag_norm =
        fragmentation::score(switch_count, tracked_seconds.min(u32::MAX as u64) as u32);
    let fragmentation_score: u8 = (frag_norm * 100.0).clamp(0.0, 100.0).round() as u8;

    // 5) focus_pct: percent of tracked seconds in focus/work categories.
    //    LEFT JOIN app_category_map covers the case where the event was
    //    inserted before a mapping existed (e.g. early 'jadons-excuses'
    //    rows pre-v2 migration).
    let focus_row = sqlx::query(
        "SELECT \
            COALESCE(SUM( \
                COALESCE(ae.duration_seconds, (? - ae.started_at) / 1000) \
            ), 0) AS focus_secs \
         FROM activity_events ae \
         LEFT JOIN app_category_map m \
             ON m.pattern = ae.app_name AND m.pattern_type = 'app' \
         LEFT JOIN categories c \
             ON c.id = COALESCE(ae.category_id, m.category_id) \
         WHERE ae.started_at >= ? \
           AND c.productivity_level IN ('focus', 'work')",
    )
    .bind(now)
    .bind(start)
    .fetch_one(&pool)
    .await
    .map_err(|e| e.to_string())?;
    let focus_secs = decode_i64(&focus_row, "focus_secs", 0).max(0) as u64;
    let focus_pct: u8 = if tracked_seconds == 0 {
        0
    } else {
        ((focus_secs as f64 / tracked_seconds as f64) * 100.0)
            .clamp(0.0, 100.0)
            .round() as u8
    };

    let focus_score = compute_focus_score(
        engagement_avg,
        fragmentation_score,
        focus_pct,
        total_samples,
        tracked_seconds,
    );

    Ok(TodayOverview {
        tracked_seconds,
        active_seconds,
        idle_seconds,
        intense_seconds,
        switch_count,
        fragmentation_score,
        focus_score,
        engagement_sample_count: total_samples,
    })
}

pub async fn get_top_apps_today(
    app: &AppHandle,
    limit: u32,
) -> Result<Vec<TopApp>, String> {
    let pool = sqlite_pool(app).await?;
    let start = start_of_today_utc_ms();
    let now = now_ms();
    // Hard cap so a misbehaving frontend can't ask for the universe.
    let limit = limit.clamp(1, 100) as i64;

    // Two-step: first the per-app aggregate (cheap, indexed), then a
    // joined engagement aggregate keyed on app_name. Doing both as
    // separate CTEs lets SQLite use the started_at index on each.
    let rows = sqlx::query(
        "WITH per_app AS ( \
            SELECT \
                ae.app_name, \
                SUM(COALESCE(ae.duration_seconds, (? - ae.started_at) / 1000)) AS total_seconds, \
                COUNT(*) AS event_count, \
                MAX(ae.category_id) AS event_category_id \
            FROM activity_events ae \
            WHERE ae.started_at >= ? \
            GROUP BY ae.app_name \
         ), \
         per_app_engagement AS ( \
            SELECT \
                ae.app_name, \
                SUM(CASE WHEN es.engagement_score > 0 THEN 1 ELSE 0 END) AS active_samples, \
                CAST(AVG(es.engagement_score) AS REAL) AS avg_engagement \
            FROM activity_events ae \
            JOIN engagement_samples es ON es.activity_event_id = ae.id \
            WHERE ae.started_at >= ? \
            GROUP BY ae.app_name \
         ) \
         SELECT \
            p.app_name, \
            p.total_seconds, \
            p.event_count, \
            COALESCE(p.event_category_id, m.category_id) AS resolved_category_id, \
            c.name AS category_name, \
            c.color AS category_color, \
            COALESCE(e.active_samples, 0) AS active_samples, \
            COALESCE(e.avg_engagement, 0.0) AS avg_engagement \
         FROM per_app p \
         LEFT JOIN app_category_map m \
            ON m.pattern = p.app_name AND m.pattern_type = 'app' \
         LEFT JOIN categories c \
            ON c.id = COALESCE(p.event_category_id, m.category_id) \
         LEFT JOIN per_app_engagement e ON e.app_name = p.app_name \
         ORDER BY p.total_seconds DESC \
         LIMIT ?",
    )
    .bind(now)
    .bind(start)
    .bind(start)
    .bind(limit)
    .fetch_all(&pool)
    .await
    .map_err(|e| e.to_string())?;

    let apps = rows
        .iter()
        .map(|r| {
            let total_seconds = decode_i64(r, "total_seconds", 0);
            let event_count = decode_i64(r, "event_count", 0);
            let active_samples = decode_i64(r, "active_samples", 0);
            let avg_engagement = decode_avg_score_u8(r, "avg_engagement");
            TopApp {
                app_name: decode_string(r, "app_name", "(unknown)"),
                total_seconds,
                event_count: event_count.clamp(0, u32::MAX as i64) as u32,
                category_id: decode_opt_i64(r, "resolved_category_id"),
                category_name: decode_opt_string(r, "category_name"),
                category_color: decode_opt_string(r, "category_color"),
                active_seconds: (active_samples.max(0) as u64)
                    .saturating_mul(SECS_PER_SAMPLE),
                avg_engagement,
            }
        })
        .collect();

    Ok(apps)
}

fn browser_apps_sql_in_clause() -> String {
    BROWSER_APPS
        .iter()
        .map(|name| format!("'{}'", name.replace('\'', "''")))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Mixed apps + domains for “Where your day went”.
pub async fn get_top_activity_today(
    app: &AppHandle,
    limit: u32,
) -> Result<Vec<TopActivity>, String> {
    let pool = sqlite_pool(app).await?;
    let start = start_of_today_utc_ms();
    let now = now_ms();
    let limit = limit.clamp(1, 100) as i64;
    let in_clause = browser_apps_sql_in_clause();

    // Duration sums must never JOIN engagement_samples (that would multiply
    // each event's duration). We aggregate durations from `activity_events`
    // alone, then attach engagement rollups in separate subqueries.
    let sql = format!(
        "SELECT \
           x.kind AS kind, \
           x.name AS name, \
           x.icon_hint AS icon_hint, \
           x.total_seconds AS total_seconds, \
           COALESCE(x.active_samples, 0) AS active_samples, \
           CAST(COALESCE(x.avg_engagement, 0.0) AS REAL) AS avg_engagement, \
           x.cat_id AS cat_id, \
           c.name AS category_name, \
           c.color AS category_color \
         FROM ( \
           SELECT \
             'website' AS kind, \
             d.name AS name, \
             d.name AS icon_hint, \
             d.total_seconds AS total_seconds, \
             e.active_samples AS active_samples, \
             e.avg_engagement AS avg_engagement, \
             d.cat_id AS cat_id \
           FROM ( \
             SELECT \
               ae.browser_domain AS name, \
               SUM(COALESCE(ae.duration_seconds, (? - ae.started_at) / 1000)) AS total_seconds, \
               MAX(ae.category_id) AS cat_id \
             FROM activity_events ae \
             WHERE ae.started_at >= ? AND ae.browser_domain IS NOT NULL \
               AND LENGTH(TRIM(ae.browser_domain)) > 0 \
             GROUP BY ae.browser_domain \
           ) d \
           LEFT JOIN ( \
             SELECT \
               ae.browser_domain AS name, \
               COALESCE(SUM(CASE WHEN es.engagement_score > 0 THEN 1 ELSE 0 END), 0) AS active_samples, \
               AVG(es.engagement_score) AS avg_engagement \
             FROM activity_events ae \
             INNER JOIN engagement_samples es ON es.activity_event_id = ae.id \
             WHERE ae.started_at >= ? AND ae.browser_domain IS NOT NULL \
               AND LENGTH(TRIM(ae.browser_domain)) > 0 \
             GROUP BY ae.browser_domain \
           ) e ON e.name = d.name \
           UNION ALL \
           SELECT \
             'app' AS kind, \
             d.name AS name, \
             d.name AS icon_hint, \
             d.total_seconds AS total_seconds, \
             e.active_samples AS active_samples, \
             e.avg_engagement AS avg_engagement, \
             d.cat_id AS cat_id \
           FROM ( \
             SELECT \
               ae.app_name AS name, \
               SUM(COALESCE(ae.duration_seconds, (? - ae.started_at) / 1000)) AS total_seconds, \
               MAX(ae.category_id) AS cat_id \
             FROM activity_events ae \
             WHERE ae.started_at >= ? AND ae.app_name NOT IN ({in_clause}) \
             GROUP BY ae.app_name \
           ) d \
           LEFT JOIN ( \
             SELECT \
               ae.app_name AS name, \
               COALESCE(SUM(CASE WHEN es.engagement_score > 0 THEN 1 ELSE 0 END), 0) AS active_samples, \
               AVG(es.engagement_score) AS avg_engagement \
             FROM activity_events ae \
             INNER JOIN engagement_samples es ON es.activity_event_id = ae.id \
             WHERE ae.started_at >= ? AND ae.app_name NOT IN ({in_clause}) \
             GROUP BY ae.app_name \
           ) e ON e.name = d.name \
           UNION ALL \
           SELECT \
             'app' AS kind, \
             'Browser (untracked)' AS name, \
             CAST(NULL AS TEXT) AS icon_hint, \
             d.total_seconds AS total_seconds, \
             e.active_samples AS active_samples, \
             e.avg_engagement AS avg_engagement, \
             d.cat_id AS cat_id \
           FROM ( \
             SELECT \
               SUM(COALESCE(ae.duration_seconds, (? - ae.started_at) / 1000)) AS total_seconds, \
               MAX(ae.category_id) AS cat_id \
             FROM activity_events ae \
             WHERE ae.started_at >= ? AND ae.app_name IN ({in_clause}) \
               AND ae.browser_domain IS NULL \
           ) d \
           LEFT JOIN ( \
             SELECT \
               COALESCE(SUM(CASE WHEN es.engagement_score > 0 THEN 1 ELSE 0 END), 0) AS active_samples, \
               AVG(es.engagement_score) AS avg_engagement \
             FROM activity_events ae \
             INNER JOIN engagement_samples es ON es.activity_event_id = ae.id \
             WHERE ae.started_at >= ? AND ae.app_name IN ({in_clause}) \
               AND ae.browser_domain IS NULL \
           ) e ON 1=1 \
         ) x \
         LEFT JOIN categories c ON c.id = x.cat_id \
         ORDER BY x.total_seconds DESC \
         LIMIT ?"
    );

    let rows = sqlx::query(&sql)
        .bind(now)
        .bind(start)
        .bind(start)
        .bind(now)
        .bind(start)
        .bind(start)
        .bind(now)
        .bind(start)
        .bind(start)
        .bind(limit)
        .fetch_all(&pool)
        .await
        .map_err(|e| e.to_string())?;

    Ok(rows
        .iter()
        .filter_map(|r| {
            let total_seconds = decode_i64(r, "total_seconds", 0);
            if total_seconds <= 0 {
                return None;
            }
            let active_samples = decode_i64(r, "active_samples", 0);
            Some(TopActivity {
                kind: decode_string(r, "kind", "app"),
                name: decode_string(r, "name", ""),
                icon_hint: decode_opt_string(r, "icon_hint"),
                total_seconds,
                active_seconds: (active_samples.max(0) as u64).saturating_mul(SECS_PER_SAMPLE) as i64,
                avg_engagement: decode_avg_score_u8(r, "avg_engagement"),
                category_name: decode_opt_string(r, "category_name"),
                category_color: decode_opt_string(r, "category_color"),
            })
        })
        .collect())
}

pub async fn get_hourly_engagement_today(
    app: &AppHandle,
) -> Result<Vec<HourPoint>, String> {
    let pool = sqlite_pool(app).await?;
    let start = start_of_today_utc_ms();
    // Cap at 24h after midnight so a clock-skew or imported old sample
    // can't bleed into hour 24+.
    let end = start + 24 * 3_600_000;

    // active_minutes: COUNT DISTINCT minute-buckets that contain ≥1
    // non-idle sample. With 10s samples that's at most 6 samples per
    // minute, so the cardinality is bounded at 60 per hour and the
    // expression is cheap.
    let rows = sqlx::query(
        "SELECT \
            CAST((sampled_at - ?) / 3600000 AS INTEGER) AS hour, \
            CAST(AVG(engagement_score) AS REAL) AS avg_score, \
            COUNT(DISTINCT CASE WHEN engagement_score > 0 \
                                 THEN sampled_at / 60000 ELSE NULL END) \
                AS active_minutes \
         FROM engagement_samples \
         WHERE sampled_at >= ? AND sampled_at < ? \
         GROUP BY hour \
         ORDER BY hour",
    )
    .bind(start)
    .bind(start)
    .bind(end)
    .fetch_all(&pool)
    .await
    .map_err(|e| e.to_string())?;

    // Pre-fill 24 zero hours so the chart always renders an even axis.
    let mut hours: Vec<HourPoint> = (0..24u8)
        .map(|h| HourPoint {
            hour: h,
            avg_engagement: 0,
            active_minutes: 0,
        })
        .collect();

    for r in rows {
        let hour = decode_i64(&r, "hour", -1);
        let active_minutes = decode_i64(&r, "active_minutes", 0);
        if (0..24).contains(&hour) {
            let idx = hour as usize;
            hours[idx].avg_engagement = decode_avg_score_u8(&r, "avg_score");
            hours[idx].active_minutes = active_minutes.clamp(0, 60) as u8;
        }
    }

    Ok(hours)
}

/// Longest focus streak (seconds) from today's UTC `daily_summaries` row.
pub async fn longest_streak_seconds_today_utc(app: &AppHandle) -> u64 {
    let Ok(pool) = sqlite_pool(app).await else {
        return 0;
    };
    let date = Utc::now().format("%Y-%m-%d").to_string();
    let v: Option<i64> = sqlx::query_scalar(
        "SELECT longest_streak_seconds FROM daily_summaries WHERE date = ?",
    )
    .bind(&date)
    .fetch_optional(&pool)
    .await
    .ok()
    .flatten();
    v.unwrap_or(0).max(0) as u64
}

// --- tests ---------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn focus_score_zero_when_no_data() {
        // No samples → 0 even if other inputs look "good".
        assert_eq!(compute_focus_score(80, 10, 90, 0, 1000), 0);
        // No tracked time → 0.
        assert_eq!(compute_focus_score(80, 10, 90, 100, 0), 0);
    }

    #[test]
    fn focus_score_perfect_day() {
        // Maxed-out engagement, zero fragmentation, all focus categories.
        let s = compute_focus_score(100, 0, 100, 100, 3600);
        // 0.5*100 + 0.3*100 + 0.2*100 = 100
        assert_eq!(s, 100);
    }

    #[test]
    fn focus_score_chaotic_day() {
        // Decent engagement but fragmentation through the roof, and
        // none of the time was in focus/work categories.
        let s = compute_focus_score(60, 100, 0, 100, 3600);
        // 0.5*60 + 0.3*0 + 0.2*0 = 30
        assert_eq!(s, 30);
    }

    #[test]
    fn focus_score_balanced_day() {
        // engagement 50, frag 40, focus_pct 70
        // 0.5*50 + 0.3*60 + 0.2*70 = 25 + 18 + 14 = 57
        let s = compute_focus_score(50, 40, 70, 100, 3600);
        assert_eq!(s, 57);
    }

    #[test]
    fn focus_score_clamps_at_100() {
        // Pathological inputs (out-of-range u8) can't actually happen
        // because each input is u8 with values 0..=100, but verify the
        // clamp anyway.
        let s = compute_focus_score(200, 0, 200, 1, 1);
        assert_eq!(s, 100);
    }
}
