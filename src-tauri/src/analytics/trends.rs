//! Trends API — reads `daily_summaries` + engagement history for charts.

use serde::{Deserialize, Serialize};
use sqlx::{Pool, Row, Sqlite};
use tauri::{AppHandle, Manager};
use tauri_plugin_sql::{DbInstances, DbPool};

use crate::analytics::summaries::DailySummaryRecord;
use crate::db::queries::DB_URL;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DateScore {
    pub date: String,
    pub score: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DateSeconds {
    pub date: String,
    pub seconds: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CategoryMixPoint {
    pub date: String,
    pub focus_seconds: u64,
    pub work_seconds: u64,
    pub distracting_seconds: u64,
    pub personal_seconds: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BestDay {
    pub date: String,
    pub focus_score: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersonalRecords {
    pub longest_streak_ever: i64,
    pub best_day: BestDay,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrendsOverview {
    pub daily_summaries: Vec<DailySummaryRecord>,
    pub focus_score_trend: Vec<DateScore>,
    pub avg_streak_trend: Vec<DateSeconds>,
    pub category_mix: Vec<CategoryMixPoint>,
    pub personal_records: PersonalRecords,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HeatmapCell {
    pub day_of_week: u8,
    pub hour: u8,
    pub avg_engagement: u8,
    pub sample_count: u64,
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

pub async fn get_trends_overview(app: &AppHandle, days: u32) -> Result<TrendsOverview, String> {
    let pool = sqlite_pool(app).await?;
    let days_i = days.clamp(1, 120) as i64;
    let cutoff_ms = chrono::Utc::now().timestamp_millis() - days_i * 86_400_000;

    let summaries = sqlx::query(
        "SELECT date, total_active_seconds, total_idle_seconds, focus_score, \
                longest_streak_seconds, total_switches, top_app, updated_at \
         FROM daily_summaries \
         ORDER BY date DESC \
         LIMIT ?",
    )
    .bind(days_i)
    .fetch_all(&pool)
    .await
    .map_err(|e| e.to_string())?;

    let daily_summaries: Vec<DailySummaryRecord> = summaries
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
        .collect();

    let mut focus_score_trend: Vec<DateScore> = daily_summaries
        .iter()
        .map(|d| DateScore {
            date: d.date.clone(),
            score: d.focus_score,
        })
        .collect();
    focus_score_trend.reverse();

    let mut avg_streak_trend: Vec<DateSeconds> = daily_summaries
        .iter()
        .map(|d| DateSeconds {
            date: d.date.clone(),
            seconds: d.longest_streak_seconds,
        })
        .collect();
    avg_streak_trend.reverse();

    let mix_rows = sqlx::query(
        "SELECT date(ae.started_at/1000, 'unixepoch') AS d, \
            SUM(CASE WHEN c.productivity_level = 'focus' THEN \
                COALESCE(ae.duration_seconds, 0) ELSE 0 END) AS fsec, \
            SUM(CASE WHEN c.productivity_level = 'work' THEN \
                COALESCE(ae.duration_seconds, 0) ELSE 0 END) AS wsec, \
            SUM(CASE WHEN c.productivity_level = 'distracting' THEN \
                COALESCE(ae.duration_seconds, 0) ELSE 0 END) AS dsec, \
            SUM(CASE WHEN c.productivity_level = 'personal' THEN \
                COALESCE(ae.duration_seconds, 0) ELSE 0 END) AS psec \
         FROM activity_events ae \
         LEFT JOIN app_category_map m ON m.pattern = ae.app_name AND m.pattern_type = 'app' \
         LEFT JOIN categories c ON c.id = COALESCE(ae.category_id, m.category_id) \
         WHERE ae.started_at >= ? \
         GROUP BY d \
         ORDER BY d ASC",
    )
    .bind(cutoff_ms)
    .fetch_all(&pool)
    .await
    .map_err(|e| e.to_string())?;

    let category_mix: Vec<CategoryMixPoint> = mix_rows
        .iter()
        .filter_map(|r| {
            let date: Option<String> = r.try_get("d").ok();
            let date = date?;
            Some(CategoryMixPoint {
                date,
                focus_seconds: r.get::<i64, _>("fsec").max(0) as u64,
                work_seconds: r.get::<i64, _>("wsec").max(0) as u64,
                distracting_seconds: r.get::<i64, _>("dsec").max(0) as u64,
                personal_seconds: r.get::<i64, _>("psec").max(0) as u64,
            })
        })
        .collect();

    let streak_row = sqlx::query("SELECT MAX(longest_streak_seconds) AS mx FROM daily_summaries")
        .fetch_one(&pool)
        .await
        .map_err(|e| e.to_string())?;
    let longest_streak_ever: i64 = streak_row.try_get("mx").unwrap_or(0);

    let best_row = sqlx::query(
        "SELECT date, focus_score FROM daily_summaries \
         ORDER BY focus_score DESC, date DESC LIMIT 1",
    )
    .fetch_optional(&pool)
    .await
    .map_err(|e| e.to_string())?;
    let best_day = if let Some(r) = best_row {
        BestDay {
            date: r.get("date"),
            focus_score: r.get::<i64, _>("focus_score").clamp(0, 100) as u8,
        }
    } else {
        BestDay {
            date: "".into(),
            focus_score: 0,
        }
    };

    Ok(TrendsOverview {
        daily_summaries,
        focus_score_trend,
        avg_streak_trend,
        category_mix,
        personal_records: PersonalRecords {
            longest_streak_ever,
            best_day,
        },
    })
}

pub async fn get_weekly_heatmap(app: &AppHandle) -> Result<Vec<HeatmapCell>, String> {
    let pool = sqlite_pool(app).await?;
    let start_ms = chrono::Utc::now().timestamp_millis() - 30_i64 * 86_400_000;
    let rows = sqlx::query(
        "SELECT \
            CAST(strftime('%w', es.sampled_at/1000, 'unixepoch') AS INTEGER) AS dow, \
            CAST(strftime('%H', es.sampled_at/1000, 'unixepoch') AS INTEGER) AS hr, \
            CAST(AVG(CAST(es.engagement_score AS REAL)) AS REAL) AS avg_e, \
            COUNT(*) AS cnt \
         FROM engagement_samples es \
         WHERE es.sampled_at >= ? \
         GROUP BY dow, hr",
    )
    .bind(start_ms)
    .fetch_all(&pool)
    .await
    .map_err(|e| e.to_string())?;

    Ok(rows
        .iter()
        .filter_map(|r| {
            let dow: i64 = r.try_get("dow").ok()?;
            let hr: i64 = r.try_get("hr").ok()?;
            let avg: Option<f64> = r.try_get("avg_e").ok();
            let cnt: i64 = r.try_get("cnt").unwrap_or(0);
            Some(HeatmapCell {
                day_of_week: dow.clamp(0, 6) as u8,
                hour: hr.clamp(0, 23) as u8,
                avg_engagement: avg
                    .unwrap_or(0.0)
                    .round()
                    .clamp(0.0, 100.0) as u8,
                sample_count: cnt.max(0) as u64,
            })
        })
        .collect())
}
