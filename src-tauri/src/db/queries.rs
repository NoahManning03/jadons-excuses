//! Rust query layer over the SQLite database.
//!
//! These functions are wrapped by `#[tauri::command]` shims in
//! `crate::commands` so the frontend can call them via `invoke()`. They use
//! the `sqlx` pool that `tauri-plugin-sql` is already managing — we reach
//! into its `DbInstances` state, pull out the live `Pool<Sqlite>`, and run
//! typed queries against it. That lets us share a single connection pool
//! with the frontend's `Database.load(...)` calls without opening a second
//! one against the same file (which would invite SQLite write locks).
use serde::{Deserialize, Serialize};
use sqlx::{Pool, Row, Sqlite};
use sqlx::QueryBuilder;
use tauri::{AppHandle, Manager, Runtime, State};
use tauri_plugin_sql::{DbInstances, DbPool};

pub const DB_URL: &str = "sqlite:jadons-excuses.db";

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Category {
    pub id: i64,
    pub name: String,
    pub productivity_level: String,
    pub color: Option<String>,
    pub is_default: bool,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AppMapping {
    pub id: i64,
    pub pattern: String,
    pub pattern_type: String,
    pub category_id: i64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DbHealth {
    pub db_path: String,
    pub total_activity_events: u64,
    pub total_categories: u64,
    pub total_app_mappings: u64,
    pub schema_version: u32,
}

/// Pull the live sqlite pool out of `tauri-plugin-sql`'s state. The pool is
/// cheaply cloneable (it's an `Arc` internally), so we hand back an owned
/// clone and release the read lock immediately.
async fn sqlite_pool(instances: &State<'_, DbInstances>) -> Result<Pool<Sqlite>, String> {
    let map = instances.0.read().await;
    let pool = map
        .get(DB_URL)
        .ok_or_else(|| format!("database '{DB_URL}' is not loaded yet"))?;
    match pool {
        DbPool::Sqlite(p) => Ok(p.clone()),
        #[allow(unreachable_patterns)]
        _ => Err("expected a sqlite pool".to_string()),
    }
}

fn row_to_category(row: &sqlx::sqlite::SqliteRow) -> Category {
    let is_default: i64 = row.get("is_default");
    Category {
        id: row.get("id"),
        name: row.get("name"),
        productivity_level: row.get("productivity_level"),
        color: row.get("color"),
        is_default: is_default != 0,
    }
}

fn row_to_mapping(row: &sqlx::sqlite::SqliteRow) -> AppMapping {
    AppMapping {
        id: row.get("id"),
        pattern: row.get("pattern"),
        pattern_type: row.get("pattern_type"),
        category_id: row.get("category_id"),
    }
}

const CATEGORY_SELECT: &str =
    "id, name, productivity_level, color, is_default";

/// Returns counts plus the resolved DB file path. The schema version is read
/// from sqlx's own `_sqlx_migrations` bookkeeping table — that's the table
/// the `tauri-plugin-sql` migrator populates as each `Migration` runs.
pub async fn db_health_check<R: Runtime>(
    app: &AppHandle<R>,
    instances: &State<'_, DbInstances>,
) -> Result<DbHealth, String> {
    let pool = sqlite_pool(instances).await?;

    let total_activity_events: i64 = sqlx::query("SELECT COUNT(*) AS c FROM activity_events")
        .fetch_one(&pool)
        .await
        .map_err(|e| e.to_string())?
        .get("c");
    let total_categories: i64 = sqlx::query("SELECT COUNT(*) AS c FROM categories")
        .fetch_one(&pool)
        .await
        .map_err(|e| e.to_string())?
        .get("c");
    let total_app_mappings: i64 = sqlx::query("SELECT COUNT(*) AS c FROM app_category_map")
        .fetch_one(&pool)
        .await
        .map_err(|e| e.to_string())?
        .get("c");

    // `_sqlx_migrations.version` is i64; can be NULL if the table somehow
    // ended up empty. Treat NULL as "no migrations applied" → version 0.
    let schema_version: i64 = sqlx::query("SELECT COALESCE(MAX(version), 0) AS v FROM _sqlx_migrations")
        .fetch_one(&pool)
        .await
        .map_err(|e| e.to_string())?
        .get("v");

    let app_dir = app
        .path()
        .app_config_dir()
        .map_err(|e| format!("could not resolve app_config_dir: {e}"))?;
    let db_path = app_dir
        .join("jadons-excuses.db")
        .to_string_lossy()
        .to_string();

    Ok(DbHealth {
        db_path,
        total_activity_events: total_activity_events as u64,
        total_categories: total_categories as u64,
        total_app_mappings: total_app_mappings as u64,
        schema_version: schema_version as u32,
    })
}

/// Resolve a category for an observation by walking the priority chain:
///   1. exact app match           (pattern_type = 'app')
///   2. exact domain match        (pattern_type = 'domain')
///   3. case-insensitive substring match against the window title
///      (pattern_type = 'title_contains')
/// Falls through to `None`, intentionally — the caller can then decide to
/// store the event with `category_id = NULL` or look up "Uncategorized"
/// themselves. Returning `None` here keeps the lookup honest about the fact
/// that nothing actually matched.
pub async fn lookup_category_for_app(
    app_name: String,
    browser_domain: Option<String>,
    window_title: Option<String>,
    instances: &State<'_, DbInstances>,
) -> Result<Option<Category>, String> {
    let pool = sqlite_pool(instances).await?;

    // 1. Exact app match.
    let row = sqlx::query(&format!(
        "SELECT {CATEGORY_SELECT} FROM categories c \
         JOIN app_category_map m ON m.category_id = c.id \
         WHERE m.pattern_type = 'app' AND m.pattern = ? \
         LIMIT 1"
    ))
    .bind(&app_name)
    .fetch_optional(&pool)
    .await
    .map_err(|e| e.to_string())?;
    if let Some(r) = row {
        return Ok(Some(row_to_category(&r)));
    }

    // 2. Exact domain match (only if we have a domain).
    if let Some(domain) = browser_domain.as_deref().filter(|d| !d.is_empty()) {
        let row = sqlx::query(&format!(
            "SELECT {CATEGORY_SELECT} FROM categories c \
             JOIN app_category_map m ON m.category_id = c.id \
             WHERE m.pattern_type = 'domain' AND m.pattern = ? \
             LIMIT 1"
        ))
        .bind(domain)
        .fetch_optional(&pool)
        .await
        .map_err(|e| e.to_string())?;
        if let Some(r) = row {
            return Ok(Some(row_to_category(&r)));
        }
    }

    // 3. title_contains match. We let SQLite do the LIKE work, with the
    //    pattern surrounded by '%' so 'morning standup' matches 'standup'.
    //    LIKE is ASCII-case-insensitive in SQLite by default, which is what
    //    we want for window titles like "Inbox - Slack" vs "inbox".
    if let Some(title) = window_title.as_deref().filter(|t| !t.is_empty()) {
        let row = sqlx::query(&format!(
            "SELECT {CATEGORY_SELECT} FROM categories c \
             JOIN app_category_map m ON m.category_id = c.id \
             WHERE m.pattern_type = 'title_contains' \
               AND ? LIKE '%' || m.pattern || '%' \
             LIMIT 1"
        ))
        .bind(title)
        .fetch_optional(&pool)
        .await
        .map_err(|e| e.to_string())?;
        if let Some(r) = row {
            return Ok(Some(row_to_category(&r)));
        }
    }

    Ok(None)
}

pub async fn list_categories(
    instances: &State<'_, DbInstances>,
) -> Result<Vec<Category>, String> {
    let pool = sqlite_pool(instances).await?;
    let rows = sqlx::query(&format!(
        "SELECT {CATEGORY_SELECT} FROM categories ORDER BY id"
    ))
    .fetch_all(&pool)
    .await
    .map_err(|e| e.to_string())?;
    Ok(rows.iter().map(row_to_category).collect())
}

pub async fn list_app_mappings(
    category_id: Option<i64>,
    instances: &State<'_, DbInstances>,
) -> Result<Vec<AppMapping>, String> {
    let pool = sqlite_pool(instances).await?;

    // Two query shapes; binding `Option<i64>` directly would need the
    // matching column to be nullable, which it isn't, so we branch.
    let rows = match category_id {
        Some(cid) => {
            sqlx::query(
                "SELECT id, pattern, pattern_type, category_id \
                 FROM app_category_map \
                 WHERE category_id = ? \
                 ORDER BY pattern_type, pattern",
            )
            .bind(cid)
            .fetch_all(&pool)
            .await
        }
        None => {
            sqlx::query(
                "SELECT id, pattern, pattern_type, category_id \
                 FROM app_category_map \
                 ORDER BY category_id, pattern_type, pattern",
            )
            .fetch_all(&pool)
            .await
        }
    }
    .map_err(|e| e.to_string())?;

    Ok(rows.iter().map(row_to_mapping).collect())
}

// ---------------------------------------------------------------------------
// Activity timeline (search / filter / pagination)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ActivityFilters {
    /// Inclusive lower bound on `started_at` (ms).
    pub date_start: Option<i64>,
    /// Inclusive upper bound on `started_at` (ms).
    pub date_end: Option<i64>,
    /// Case-insensitive `LIKE` on `app_name` and `window_title`.
    pub search: Option<String>,
    pub category_ids: Option<Vec<i64>>,
    pub min_duration_seconds: Option<u32>,
    pub limit: Option<u32>,
    pub offset: Option<u32>,
    /// `"event"` (default), `"app"`, or `"domain"`.
    pub group_by: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActivityEventWithCategory {
    pub id: i64,
    pub app_name: String,
    pub window_title: Option<String>,
    pub browser_url: Option<String>,
    pub browser_domain: Option<String>,
    pub category_id: Option<i64>,
    pub started_at: i64,
    pub ended_at: Option<i64>,
    pub duration_seconds: Option<i64>,
    pub category_name: Option<String>,
    pub category_color: Option<String>,
    pub avg_engagement: u8,
}

fn row_activity_event_with_cat(row: &sqlx::sqlite::SqliteRow) -> ActivityEventWithCategory {
    let avg_raw: Option<f64> = row.try_get("avg_engagement_score").ok().flatten();
    let avg_engagement = avg_raw
        .unwrap_or(0.0)
        .round()
        .clamp(0.0, 100.0) as u8;
    ActivityEventWithCategory {
        id: row.get("id"),
        app_name: row.get("app_name"),
        window_title: row.get("window_title"),
        browser_url: row.get("browser_url"),
        browser_domain: row.get("browser_domain"),
        category_id: row.get("category_id"),
        started_at: row.get("started_at"),
        ended_at: row.get("ended_at"),
        duration_seconds: row.get("duration_seconds"),
        category_name: row.get("category_name"),
        category_color: row.get("category_color"),
        avg_engagement,
    }
}

fn push_activity_filters(
    qb: &mut QueryBuilder<'_, Sqlite>,
    filters: &ActivityFilters,
    now_ms: i64,
) {
    qb.push(" WHERE 1=1 ");
    if let Some(ds) = filters.date_start {
        qb.push(" AND ae.started_at >= ");
        qb.push_bind(ds);
    }
    if let Some(de) = filters.date_end {
        qb.push(" AND ae.started_at <= ");
        qb.push_bind(de);
    }
    if let Some(ref q) = filters.search {
        let trimmed = q.trim();
        if !trimmed.is_empty() {
            let like = format!("%{trimmed}%");
            qb.push(" AND (LOWER(ae.app_name) LIKE LOWER(");
            qb.push_bind(like.clone());
            qb.push(") OR LOWER(COALESCE(ae.window_title, '')) LIKE LOWER(");
            qb.push_bind(like);
            qb.push(")) ");
        }
    }
    if let Some(ref ids) = filters.category_ids {
        if !ids.is_empty() {
            qb.push(" AND COALESCE(ae.category_id, m.category_id) IN (");
            {
                let mut sep = qb.separated(", ");
                for id in ids {
                    sep.push_bind(*id);
                }
            }
            qb.push(") ");
        }
    }
    if let Some(min_s) = filters.min_duration_seconds.filter(|&m| m > 0) {
        qb.push(" AND COALESCE(ae.duration_seconds, (");
        qb.push_bind(now_ms);
        qb.push(" - ae.started_at) / 1000) >= ");
        qb.push_bind(min_s as i64);
    }
}

const ACTIVITY_FROM: &str = " FROM activity_events ae \
     LEFT JOIN app_category_map m ON m.pattern = ae.app_name AND m.pattern_type = 'app' \
     LEFT JOIN categories c ON c.id = COALESCE(ae.category_id, m.category_id) \
     LEFT JOIN ( \
         SELECT activity_event_id, CAST(AVG(CAST(engagement_score AS REAL)) AS REAL) AS avg_score \
         FROM engagement_samples GROUP BY activity_event_id \
     ) eng ON eng.activity_event_id = ae.id ";

fn activity_group_mode(filters: &ActivityFilters) -> &str {
    match filters.group_by.as_deref() {
        Some("app") => "app",
        Some("domain") => "domain",
        _ => "event",
    }
}

pub async fn get_activity_events(
    instances: &State<'_, DbInstances>,
    filters: ActivityFilters,
    now_ms: i64,
) -> Result<Vec<ActivityEventWithCategory>, String> {
    let pool = sqlite_pool(instances).await?;
    let limit = filters.limit.unwrap_or(100).clamp(1, 500) as i64;
    let offset = filters.offset.unwrap_or(0) as i64;

    match activity_group_mode(&filters) {
        "event" => {
            let mut qb = QueryBuilder::new(
                "SELECT ae.id, ae.app_name, ae.window_title, ae.browser_url, ae.browser_domain, \
                 ae.category_id, ae.started_at, ae.ended_at, ae.duration_seconds, \
                 c.name AS category_name, c.color AS category_color, \
                 CAST(COALESCE(eng.avg_score, 0.0) AS REAL) AS avg_engagement_score ",
            );
            qb.push(ACTIVITY_FROM);
            push_activity_filters(&mut qb, &filters, now_ms);
            qb.push(" ORDER BY ae.started_at DESC LIMIT ");
            qb.push_bind(limit);
            qb.push(" OFFSET ");
            qb.push_bind(offset);

            let rows = qb
                .build()
                .fetch_all(&pool)
                .await
                .map_err(|e| e.to_string())?;
            Ok(rows.iter().map(row_activity_event_with_cat).collect())
        }
        "app" => {
            let mut qb = QueryBuilder::new(
                "SELECT agg.id, agg.app_name, agg.window_title, agg.browser_url, agg.browser_domain, \
                 agg.category_id, agg.started_at, agg.ended_at, agg.duration_seconds, \
                 cat.name AS category_name, cat.color AS category_color, \
                 CAST(COALESCE(agg.avg_engagement_score, 0.0) AS REAL) AS avg_engagement_score \
                 FROM ( \
                   SELECT \
                     MAX(ae.id) AS id, \
                     ae.app_name AS app_name, \
                     CAST(NULL AS TEXT) AS window_title, \
                     CAST(NULL AS TEXT) AS browser_url, \
                     CAST(NULL AS TEXT) AS browser_domain, \
                     MAX(COALESCE(ae.category_id, m.category_id)) AS category_id, \
                     MIN(ae.started_at) AS started_at, \
                     MAX(ae.ended_at) AS ended_at, \
                     SUM(COALESCE(ae.duration_seconds, (",
            );
            qb.push_bind(now_ms);
            qb.push(" - ae.started_at) / 1000)) AS duration_seconds, \
                     AVG((SELECT AVG(CAST(es.engagement_score AS REAL)) FROM engagement_samples es \
                          WHERE es.activity_event_id = ae.id)) AS avg_engagement_score \
                   FROM activity_events ae \
                   LEFT JOIN app_category_map m ON m.pattern = ae.app_name AND m.pattern_type = 'app' ",
            );
            push_activity_filters(&mut qb, &filters, now_ms);
            qb.push(" GROUP BY ae.app_name \
                 ) agg \
                 LEFT JOIN categories cat ON cat.id = agg.category_id \
                 ORDER BY agg.duration_seconds DESC, agg.app_name ASC LIMIT ",
            );
            qb.push_bind(limit);
            qb.push(" OFFSET ");
            qb.push_bind(offset);

            let rows = qb
                .build()
                .fetch_all(&pool)
                .await
                .map_err(|e| e.to_string())?;
            Ok(rows.iter().map(row_activity_event_with_cat).collect())
        }
        "domain" => {
            let mut qb = QueryBuilder::new(
                "SELECT agg.id, agg.app_name, agg.window_title, agg.browser_url, agg.browser_domain, \
                 agg.category_id, agg.started_at, agg.ended_at, agg.duration_seconds, \
                 cat.name AS category_name, cat.color AS category_color, \
                 CAST(COALESCE(agg.avg_engagement_score, 0.0) AS REAL) AS avg_engagement_score \
                 FROM ( \
                   SELECT \
                     MAX(ae.id) AS id, \
                     ae.browser_domain AS app_name, \
                     CAST(NULL AS TEXT) AS window_title, \
                     CAST(NULL AS TEXT) AS browser_url, \
                     ae.browser_domain AS browser_domain, \
                     MAX(COALESCE(ae.category_id, m.category_id)) AS category_id, \
                     MAX(ae.started_at) AS started_at, \
                     MAX(ae.ended_at) AS ended_at, \
                     SUM(COALESCE(ae.duration_seconds, (",
            );
            qb.push_bind(now_ms);
            qb.push(" - ae.started_at) / 1000)) AS duration_seconds, \
                     AVG((SELECT AVG(CAST(es.engagement_score AS REAL)) FROM engagement_samples es \
                          WHERE es.activity_event_id = ae.id)) AS avg_engagement_score \
                   FROM activity_events ae \
                   LEFT JOIN app_category_map m ON m.pattern = ae.app_name AND m.pattern_type = 'app' ",
            );
            push_activity_filters(&mut qb, &filters, now_ms);
            qb.push(
                " AND ae.browser_domain IS NOT NULL AND LENGTH(TRIM(ae.browser_domain)) > 0 \
                  GROUP BY ae.browser_domain \
                 ) agg \
                 LEFT JOIN categories cat ON cat.id = agg.category_id \
                 ORDER BY agg.duration_seconds DESC, agg.browser_domain ASC LIMIT ",
            );
            qb.push_bind(limit);
            qb.push(" OFFSET ");
            qb.push_bind(offset);

            let rows = qb
                .build()
                .fetch_all(&pool)
                .await
                .map_err(|e| e.to_string())?;
            Ok(rows.iter().map(row_activity_event_with_cat).collect())
        }
        _ => Err("invalid group_by".into()),
    }
}

pub async fn get_activity_event_count(
    instances: &State<'_, DbInstances>,
    filters: ActivityFilters,
    now_ms: i64,
) -> Result<u64, String> {
    let pool = sqlite_pool(instances).await?;

    match activity_group_mode(&filters) {
        "event" => {
            let mut qb = QueryBuilder::new("SELECT COUNT(*) AS c ");
            qb.push(ACTIVITY_FROM);
            push_activity_filters(&mut qb, &filters, now_ms);
            let row = qb
                .build()
                .fetch_one(&pool)
                .await
                .map_err(|e| e.to_string())?;
            let c: i64 = row.get("c");
            Ok(c.max(0) as u64)
        }
        "app" => {
            let mut qb = QueryBuilder::new(
                "SELECT COUNT(*) AS c FROM ( SELECT ae.app_name FROM activity_events ae \
                 LEFT JOIN app_category_map m ON m.pattern = ae.app_name AND m.pattern_type = 'app' ",
            );
            push_activity_filters(&mut qb, &filters, now_ms);
            qb.push(" GROUP BY ae.app_name ) t ");
            let row = qb
                .build()
                .fetch_one(&pool)
                .await
                .map_err(|e| e.to_string())?;
            let c: i64 = row.get("c");
            Ok(c.max(0) as u64)
        }
        "domain" => {
            let mut qb = QueryBuilder::new(
                "SELECT COUNT(*) AS c FROM ( SELECT ae.browser_domain FROM activity_events ae \
                 LEFT JOIN app_category_map m ON m.pattern = ae.app_name AND m.pattern_type = 'app' ",
            );
            push_activity_filters(&mut qb, &filters, now_ms);
            qb.push(
                " AND ae.browser_domain IS NOT NULL AND LENGTH(TRIM(ae.browser_domain)) > 0 \
                  GROUP BY ae.browser_domain ) t ",
            );
            let row = qb
                .build()
                .fetch_one(&pool)
                .await
                .map_err(|e| e.to_string())?;
            let c: i64 = row.get("c");
            Ok(c.max(0) as u64)
        }
        _ => Err("invalid group_by".into()),
    }
}

/// Underlying events for an aggregate row (expand drill-down).
/// Applies the same filters as the grouped activity list (search, categories,
/// min duration, date range). Pagination / `group_by` on `filters` are ignored.
pub async fn get_aggregate_event_details(
    instances: &State<'_, DbInstances>,
    name: String,
    kind: String,
    mut filters: ActivityFilters,
    now_ms: i64,
) -> Result<Vec<ActivityEventWithCategory>, String> {
    filters.limit = None;
    filters.offset = None;
    filters.group_by = None;

    let pool = sqlite_pool(instances).await?;

    let mut qb = QueryBuilder::new(
        "SELECT ae.id, ae.app_name, ae.window_title, ae.browser_url, ae.browser_domain, \
         ae.category_id, ae.started_at, ae.ended_at, ae.duration_seconds, \
         c.name AS category_name, c.color AS category_color, \
         CAST(COALESCE(eng.avg_score, 0.0) AS REAL) AS avg_engagement_score ",
    );
    qb.push(ACTIVITY_FROM);
    push_activity_filters(&mut qb, &filters, now_ms);
    if kind == "app" {
        qb.push(" AND ae.app_name = ");
        qb.push_bind(name);
    } else if kind == "domain" {
        qb.push(" AND ae.browser_domain = ");
        qb.push_bind(name);
    } else {
        return Err("kind must be app or domain".into());
    }
    qb.push(" ORDER BY ae.started_at DESC LIMIT 100 ");

    let rows = qb
        .build()
        .fetch_all(&pool)
        .await
        .map_err(|e| e.to_string())?;
    Ok(rows.iter().map(row_activity_event_with_cat).collect())
}

// ---------------------------------------------------------------------------
// App settings (key-value in `app_settings`)
// ---------------------------------------------------------------------------

pub async fn get_app_setting(
    instances: &State<'_, DbInstances>,
    key: &str,
) -> Result<Option<String>, String> {
    let pool = sqlite_pool(instances).await?;
    let v = sqlx::query_scalar::<_, String>(
        "SELECT value FROM app_settings WHERE key = ? LIMIT 1",
    )
    .bind(key)
    .fetch_optional(&pool)
    .await
    .map_err(|e| e.to_string())?;
    Ok(v)
}

pub async fn set_app_setting(
    instances: &State<'_, DbInstances>,
    key: &str,
    value: &str,
) -> Result<(), String> {
    let pool = sqlite_pool(instances).await?;
    sqlx::query(
        "INSERT INTO app_settings (key, value) VALUES (?, ?) \
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
    )
    .bind(key)
    .bind(value)
    .execute(&pool)
    .await
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// Upsert app mapping and optionally rewrite historic `activity_events`.
/// Returns rows touched: `UPDATE activity_events` count + 1 if mapping upsert ran.
pub async fn recategorize_app(
    instances: &State<'_, DbInstances>,
    app_name: String,
    category_id: i64,
    retroactive: bool,
) -> Result<u32, String> {
    let pool = sqlite_pool(instances).await?;
    let mut total: u32 = 0;
    if retroactive {
        let res = sqlx::query(
            "UPDATE activity_events SET category_id = ? WHERE app_name = ?",
        )
        .bind(category_id)
        .bind(&app_name)
        .execute(&pool)
        .await
        .map_err(|e| e.to_string())?;
        total = total.saturating_add(res.rows_affected() as u32);
    }
    sqlx::query(
        "INSERT INTO app_category_map (pattern, pattern_type, category_id) \
         VALUES (?, 'app', ?) \
         ON CONFLICT(pattern, pattern_type) DO UPDATE SET category_id = excluded.category_id",
    )
    .bind(&app_name)
    .bind(category_id)
    .execute(&pool)
    .await
    .map_err(|e| e.to_string())?;
    total = total.saturating_add(1);
    Ok(total)
}
