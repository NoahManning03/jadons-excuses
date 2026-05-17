//! Rule-based insights (no LLM). Writes rows into `insights`.

use chrono::Utc;
use serde::{Deserialize, Serialize};
use sqlx::{Pool, Row, Sqlite};
use tauri::{AppHandle, Manager};
use tauri_plugin_sql::{DbInstances, DbPool};

use crate::db::queries::DB_URL;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Insight {
    pub id: i64,
    pub created_at: i64,
    pub title: String,
    pub body: String,
    pub severity: String,
    pub tag: String,
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

async fn upsert_insight(
    pool: &Pool<Sqlite>,
    tag: &str,
    title: &str,
    body: &str,
    severity: &str,
) -> Result<(), String> {
    sqlx::query("DELETE FROM insights WHERE tag = ?")
        .bind(tag)
        .execute(pool)
        .await
        .map_err(|e| e.to_string())?;
    let now = Utc::now().timestamp_millis();
    sqlx::query(
        "INSERT INTO insights (created_at, title, body, severity, tag) \
         VALUES (?, ?, ?, ?, ?)",
    )
    .bind(now)
    .bind(title)
    .bind(body)
    .bind(severity)
    .bind(tag)
    .execute(pool)
    .await
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// Generate / refresh deterministic insights for the last 14–30 days window.
pub async fn generate_insights(app: &AppHandle) -> Result<(), String> {
    let pool = sqlite_pool(app).await?;
    let start_ms = Utc::now().timestamp_millis() - 14_i64 * 86_400_000;

    // Best hour (highest avg engagement).
    if let Some(row) = sqlx::query(
        "SELECT CAST(strftime('%H', sampled_at/1000, 'unixepoch') AS INTEGER) AS hr, \
                CAST(AVG(CAST(engagement_score AS REAL)) AS REAL) AS av \
         FROM engagement_samples WHERE sampled_at >= ? \
         GROUP BY hr ORDER BY av DESC LIMIT 1",
    )
    .bind(start_ms)
    .fetch_optional(&pool)
    .await
    .map_err(|e| e.to_string())?
    {
        let hr: i64 = row.get("hr");
        let av: f64 = row.get::<Option<f64>, _>("av").unwrap_or(0.0);
        let overall = sqlx::query(
            "SELECT AVG(CAST(engagement_score AS REAL)) AS o \
             FROM engagement_samples WHERE sampled_at >= ?",
        )
        .bind(start_ms)
        .fetch_one(&pool)
        .await
        .map_err(|e| e.to_string())?;
        let o: f64 = overall.get::<Option<f64>, _>("o").unwrap_or(1.0).max(1.0);
        let lift = ((av / o - 1.0) * 100.0).round().max(0.0) as i64;
        let title = "Your sharpest hour".to_string();
        let body = format!(
            "Between {hr}:00–{hr}:59 UTC you averaged {av:.0}/100 engagement — about {lift}% above your 14-day baseline. Stack important work there when you can.",
            hr = hr,
            av = av,
            lift = lift,
        );
        upsert_insight(&pool, "best_hour", &title, &body, "info").await?;
    }

    // Worst hour.
    if let Some(row) = sqlx::query(
        "SELECT CAST(strftime('%H', sampled_at/1000, 'unixepoch') AS INTEGER) AS hr, \
                CAST(AVG(CAST(engagement_score AS REAL)) AS REAL) AS av \
         FROM engagement_samples WHERE sampled_at >= ? \
         GROUP BY hr ORDER BY av ASC LIMIT 1",
    )
    .bind(start_ms)
    .fetch_optional(&pool)
    .await
    .map_err(|e| e.to_string())?
    {
        let hr: i64 = row.get("hr");
        let av: f64 = row.get::<Option<f64>, _>("av").unwrap_or(0.0);
        let overall = sqlx::query(
            "SELECT AVG(CAST(engagement_score AS REAL)) AS o \
             FROM engagement_samples WHERE sampled_at >= ?",
        )
        .bind(start_ms)
        .fetch_one(&pool)
        .await
        .map_err(|e| e.to_string())?;
        let o: f64 = overall.get::<Option<f64>, _>("o").unwrap_or(1.0).max(1.0);
        let drop = ((1.0 - av / o) * 100.0).round().max(0.0) as i64;
        let title = "A predictable slump".to_string();
        let body = format!(
            "Engagement tends to dip around {hr}:00 UTC — roughly {drop}% under your 14-day average. If that matches your afternoon, front-load deep work earlier.",
        );
        upsert_insight(&pool, "worst_hour", &title, &body, "warn").await?;
    }

    // Top distraction app (last 7 days).
    let dist_start = Utc::now().timestamp_millis() - 7_i64 * 86_400_000;
    if let Some(row) = sqlx::query(
        "SELECT ae.app_name AS app, \
            SUM(COALESCE(ae.duration_seconds, 0)) AS secs \
         FROM activity_events ae \
         LEFT JOIN app_category_map m ON m.pattern = ae.app_name AND m.pattern_type = 'app' \
         LEFT JOIN categories c ON c.id = COALESCE(ae.category_id, m.category_id) \
         WHERE ae.started_at >= ? AND c.productivity_level = 'distracting' \
         GROUP BY ae.app_name ORDER BY secs DESC LIMIT 1",
    )
    .bind(dist_start)
    .fetch_optional(&pool)
    .await
    .map_err(|e| e.to_string())?
    {
        let app: String = row.get("app");
        let secs: i64 = row.get::<Option<i64>, _>("secs").unwrap_or(0).max(0);
        let per_day = secs / 60 / 7;
        let title = "Top distraction".to_string();
        let body = format!(
            "Roughly {per_day} minutes per day in {app} over the last week — the single biggest distracting sink we see.",
        );
        upsert_insight(&pool, "top_distraction", &title, &body, "danger").await?;
    }

    // First-week style milestone when enough days in daily_summaries.
    let days: i64 = sqlx::query("SELECT COUNT(*) AS c FROM daily_summaries")
        .fetch_one(&pool)
        .await
        .map_err(|e| e.to_string())?
        .get("c");
    if days >= 3 {
        let tracked = sqlx::query(
            "SELECT SUM(total_active_seconds + total_idle_seconds) AS s, \
                    AVG(CAST(focus_score AS REAL)) AS f, \
                    MAX(longest_streak_seconds) AS mx \
             FROM daily_summaries",
        )
        .fetch_one(&pool)
        .await
        .map_err(|e| e.to_string())?;
        let secs: i64 = tracked.get::<Option<i64>, _>("s").unwrap_or(0);
        let hours = secs / 3600;
        let avg_focus = tracked.get::<Option<f64>, _>("f").unwrap_or(0.0).round() as i64;
        let mx: i64 = tracked.get::<Option<i64>, _>("mx").unwrap_or(0);
        let title = "First week recap".to_string();
        let body = format!(
            "Across {days} rollup days you logged about {hours}h of tracked time, averaged a {avg_focus}/100 focus score in our daily model, and hit a longest productive streak of {mx} seconds.",
        );
        upsert_insight(&pool, "first_week", &title, &body, "info").await?;
    }

    Ok(())
}

pub async fn get_recent_insights(
    app: &AppHandle,
    limit: u32,
) -> Result<Vec<Insight>, String> {
    let pool = sqlite_pool(app).await?;
    let lim = limit.clamp(1, 200) as i64;
    let rows = sqlx::query(
        "SELECT id, created_at, title, body, severity, tag FROM insights \
         ORDER BY created_at DESC LIMIT ?",
    )
    .bind(lim)
    .fetch_all(&pool)
    .await
    .map_err(|e| e.to_string())?;
    Ok(rows
        .iter()
        .map(|r| Insight {
            id: r.get("id"),
            created_at: r.get("created_at"),
            title: r.get("title"),
            body: r.get("body"),
            severity: r.get("severity"),
            tag: r.get("tag"),
        })
        .collect())
}
