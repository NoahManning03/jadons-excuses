//! Daily rollup into `daily_summaries` for Trends / notifications.
//!
//! A background task (see `lib.rs`) calls [`rollup_stale_days`] on a timer.
//! All math is UTC calendar-day based on `started_at` / `sampled_at` ms epoch.

use chrono::{Duration, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{Pool, Row, Sqlite};
use tauri::{AppHandle, Manager};
use tauri_plugin_sql::{DbInstances, DbPool};

use crate::analytics::fragmentation;
use crate::db::queries::DB_URL;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DailySummaryRecord {
    pub date: String,
    pub total_active_seconds: u64,
    pub total_idle_seconds: u64,
    pub focus_score: u8,
    pub longest_streak_seconds: u64,
    pub total_switches: u32,
    pub top_app: Option<String>,
    pub updated_at: i64,
}

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

fn day_bounds_ms(date: &str) -> Result<(i64, i64), String> {
    let naive = chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d")
        .map_err(|e| format!("bad date {date}: {e}"))?;
    let start = Utc
        .from_utc_datetime(&naive.and_hms_opt(0, 0, 0).unwrap())
        .timestamp_millis();
    let end = Utc
        .from_utc_datetime(&naive.and_hms_opt(23, 59, 59).unwrap())
        .timestamp_millis()
        + 999;
    Ok((start, end))
}

/// Recompute one UTC calendar day and upsert `daily_summaries`.
pub async fn compute_daily_summary(pool: &Pool<Sqlite>, date: &str) -> Result<(), String> {
    let (start, end) = day_bounds_ms(date)?;
    let now_ms = Utc::now().timestamp_millis();

    // --- engagement seconds (10s buckets) --------------------------------
    let eng = sqlx::query(
        "SELECT \
            SUM(CASE WHEN es.engagement_score > 0 THEN 1 ELSE 0 END) AS active_c, \
            SUM(CASE WHEN es.engagement_score = 0 THEN 1 ELSE 0 END) AS idle_c, \
            CAST(AVG(CAST(es.engagement_score AS REAL)) AS REAL) AS avg_score \
         FROM engagement_samples es \
         JOIN activity_events ae ON ae.id = es.activity_event_id \
         WHERE es.sampled_at >= ? AND es.sampled_at <= ?",
    )
    .bind(start)
    .bind(end)
    .fetch_one(pool)
    .await
    .map_err(|e| e.to_string())?;

    let active_c: Option<i64> = eng.try_get("active_c").unwrap_or(None);
    let idle_c: Option<i64> = eng.try_get("idle_c").unwrap_or(None);
    let avg_score: Option<f64> = eng.try_get("avg_score").ok().flatten();

    let total_active_seconds = (active_c.unwrap_or(0).max(0) as u64).saturating_mul(10);
    let total_idle_seconds = (idle_c.unwrap_or(0).max(0) as u64).saturating_mul(10);

    // --- tracked seconds + switches + top app ----------------------------
    let tracked_row = sqlx::query(
        "SELECT COALESCE(SUM( \
                COALESCE(duration_seconds, (? - started_at) / 1000) \
            ), 0) AS tracked \
         FROM activity_events \
         WHERE started_at >= ? AND started_at <= ?",
    )
    .bind(now_ms)
    .bind(start)
    .bind(end)
    .fetch_one(pool)
    .await
    .map_err(|e| e.to_string())?;
    let tracked_seconds: i64 = tracked_row.get("tracked");
    let tracked_seconds = tracked_seconds.max(0) as u64;

    let switches_row = sqlx::query(
        "SELECT COUNT(*) AS c FROM tab_switches \
         WHERE switched_at >= ? AND switched_at <= ?",
    )
    .bind(start)
    .bind(end)
    .fetch_one(pool)
    .await
    .map_err(|e| e.to_string())?;
    let total_switches: i64 = switches_row.get("c");
    let total_switches = total_switches.clamp(0, u32::MAX as i64) as u32;

    let top_row = sqlx::query(
        "SELECT ae.app_name AS app, \
            SUM(COALESCE(ae.duration_seconds, (? - ae.started_at) / 1000)) AS secs \
         FROM activity_events ae \
         WHERE ae.started_at >= ? AND ae.started_at <= ? \
         GROUP BY ae.app_name \
         ORDER BY secs DESC \
         LIMIT 1",
    )
    .bind(now_ms)
    .bind(start)
    .bind(end)
    .fetch_optional(pool)
    .await
    .map_err(|e| e.to_string())?;
    let top_app: Option<String> = top_row.and_then(|r| r.try_get("app").ok());

    // --- focus score (lightweight composite for the day) -----------------
    let engagement_avg = avg_score
        .unwrap_or(0.0)
        .round()
        .clamp(0.0, 100.0) as u8;
    let frag_norm = fragmentation::score(total_switches, tracked_seconds.min(u32::MAX as u64) as u32);
    let fragmentation_score = (frag_norm * 100.0).round().clamp(0.0, 100.0) as u8;
    let focus_score = if tracked_seconds == 0 {
        0u8
    } else {
        let s = 0.55 * engagement_avg as f32 + 0.45 * (100.0 - fragmentation_score as f32);
        s.clamp(0.0, 100.0).round() as u8
    };

    // --- longest streak (focus/work only; distracting breaks) ------------
    let streak = longest_focus_streak_seconds(pool, start, end, now_ms).await?;

    let updated_at = now_ms;
    sqlx::query(
        "INSERT INTO daily_summaries ( \
            date, total_active_seconds, total_idle_seconds, focus_score, \
            longest_streak_seconds, total_switches, top_app, updated_at \
         ) VALUES (?, ?, ?, ?, ?, ?, ?, ?) \
         ON CONFLICT(date) DO UPDATE SET \
            total_active_seconds = excluded.total_active_seconds, \
            total_idle_seconds = excluded.total_idle_seconds, \
            focus_score = excluded.focus_score, \
            longest_streak_seconds = excluded.longest_streak_seconds, \
            total_switches = excluded.total_switches, \
            top_app = excluded.top_app, \
            updated_at = excluded.updated_at",
    )
    .bind(date)
    .bind(total_active_seconds as i64)
    .bind(total_idle_seconds as i64)
    .bind(focus_score as i64)
    .bind(streak as i64)
    .bind(total_switches as i64)
    .bind(top_app.clone())
    .bind(updated_at)
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;

    Ok(())
}

async fn longest_focus_streak_seconds(
    pool: &Pool<Sqlite>,
    day_start: i64,
    day_end: i64,
    now_ms: i64,
) -> Result<u64, String> {
    let rows = sqlx::query(
        "SELECT ae.id, \
            COALESCE(ae.duration_seconds, (? - ae.started_at) / 1000) AS dur, \
            COALESCE(c.productivity_level, 'neutral') AS level, \
            CAST(COALESCE(eng.avg_score, 0.0) AS REAL) AS avg_eng \
         FROM activity_events ae \
         LEFT JOIN app_category_map m ON m.pattern = ae.app_name AND m.pattern_type = 'app' \
         LEFT JOIN categories c ON c.id = COALESCE(ae.category_id, m.category_id) \
         LEFT JOIN ( \
           SELECT activity_event_id, AVG(CAST(engagement_score AS REAL)) AS avg_score \
           FROM engagement_samples GROUP BY activity_event_id \
         ) eng ON eng.activity_event_id = ae.id \
         WHERE ae.started_at >= ? AND ae.started_at <= ? \
         ORDER BY ae.started_at ASC",
    )
    .bind(now_ms)
    .bind(day_start)
    .bind(day_end)
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;

    let mut best: u64 = 0;
    let mut cur: u64 = 0;
    for r in rows {
        let dur: i64 = r.try_get("dur").unwrap_or(0).max(0);
        let level: String = r.try_get("level").unwrap_or_else(|_| "neutral".into());
        let avg_eng: f64 = r.try_get::<Option<f64>, _>("avg_eng").ok().flatten().unwrap_or(0.0);
        let is_focus = (level == "focus" || level == "work") && avg_eng > 25.0;
        let is_distraction = level == "distracting";
        if is_focus {
            cur = cur.saturating_add(dur as u64);
            best = best.max(cur);
        } else if is_distraction {
            cur = 0;
        } else {
            // neutral / personal / etc. ends the productive streak chain.
            cur = 0;
        }
    }
    Ok(best)
}

/// Roll up `today` plus any day in the last `lookback_days` missing or stale.
pub async fn rollup_stale_days(app: &AppHandle, lookback_days: i32) -> Result<(), String> {
    let pool = sqlite_pool(app).await?;
    let now = Utc::now();
    let today = now.format("%Y-%m-%d").to_string();
    compute_daily_summary(&pool, &today).await?;

    let hour_ms = 3_600_000i64;
    let cutoff = now.timestamp_millis() - hour_ms;

    for i in 1..=lookback_days {
        let d = now.date_naive() - Duration::days(i as i64);
        let ds = d.format("%Y-%m-%d").to_string();
        let row = sqlx::query(
            "SELECT updated_at FROM daily_summaries WHERE date = ? LIMIT 1",
        )
        .bind(&ds)
        .fetch_optional(&pool)
        .await
        .map_err(|e| e.to_string())?;
        let stale = match row {
            None => true,
            Some(r) => {
                let updated: i64 = r.get("updated_at");
                updated < cutoff
            }
        };
        if stale {
            compute_daily_summary(&pool, &ds).await?;
        }
    }
    Ok(())
}

pub async fn list_daily_summaries(
    app: &AppHandle,
    days: u32,
) -> Result<Vec<DailySummaryRecord>, String> {
    let pool = sqlite_pool(app).await?;
    let days = days.clamp(1, 120) as i64;
    let rows = sqlx::query(
        "SELECT date, total_active_seconds, total_idle_seconds, focus_score, \
                longest_streak_seconds, total_switches, top_app, updated_at \
         FROM daily_summaries \
         ORDER BY date DESC \
         LIMIT ?",
    )
    .bind(days)
    .fetch_all(&pool)
    .await
    .map_err(|e| e.to_string())?;

    Ok(rows
        .iter()
        .map(|r| DailySummaryRecord {
            date: r.get("date"),
            total_active_seconds: r.get::<i64, _>("total_active_seconds").max(0) as u64,
            total_idle_seconds: r.get::<i64, _>("total_idle_seconds").max(0) as u64,
            focus_score: r.get::<i64, _>("focus_score").clamp(0, 100) as u8,
            longest_streak_seconds: r.get::<i64, _>("longest_streak_seconds").max(0) as u64,
            total_switches: r.get::<i64, _>("total_switches").clamp(0, u32::MAX as i64) as u32,
            top_app: r.try_get("top_app").ok(),
            updated_at: r.get("updated_at"),
        })
        .collect())
}
