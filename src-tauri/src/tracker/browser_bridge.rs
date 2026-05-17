//! Localhost WebSocket bridge for the browser extension (Manifest V3).
//!
//! Binds **127.0.0.1:9876** only. Tab events attach URL/domain/title to
//! `activity_events`, creating rows when needed so browsing is tracked even
//! when no open browser row exists from the foreground-window tracker.

use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU32, Ordering};

use futures_util::StreamExt;
use serde::Deserialize;
use serde_json::Value;
use sqlx::{Pool, Row, Sqlite};
use sqlx::QueryBuilder;
use tauri::{AppHandle, Manager};
use tauri_plugin_sql::{DbInstances, DbPool};
use tokio::net::TcpListener;
use tokio_tungstenite::accept_async;

use crate::db::queries::{self, DB_URL};
use crate::tracker::engagement;
use crate::tracker::window::{self, BROWSER_APPS};

static SERVER_UP: AtomicBool = AtomicBool::new(false);
static CLIENT_COUNT: AtomicU32 = AtomicU32::new(0);
static LAST_MESSAGE_MS: AtomicI64 = AtomicI64::new(0);

#[derive(Debug, Clone, serde::Serialize)]
pub struct BridgeStatus {
    pub running: bool,
    pub connected_clients: u32,
    pub last_message_at: Option<i64>,
}

pub fn status() -> BridgeStatus {
    BridgeStatus {
        running: SERVER_UP.load(Ordering::Relaxed),
        connected_clients: CLIENT_COUNT.load(Ordering::Relaxed),
        last_message_at: {
            let v = LAST_MESSAGE_MS.load(Ordering::Relaxed);
            if v > 0 { Some(v) } else { None }
        },
    }
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

fn host_from_url(s: &str) -> Option<String> {
    let rest = s
        .strip_prefix("http://")
        .or_else(|| s.strip_prefix("https://"))?;
    let host = rest.split('/').next()?.split(':').next()?;
    let host = host.strip_prefix("www.").unwrap_or(host);
    if host.is_empty() {
        None
    } else {
        Some(host.to_ascii_lowercase())
    }
}

/// Block internal / non-web schemes at the bridge (defense in depth).
fn blocked_browser_scheme(url: &str) -> bool {
    let t = url.trim();
    let lower = t.to_ascii_lowercase();
    lower.starts_with("chrome://")
        || lower.starts_with("chrome-extension://")
        || lower.starts_with("devtools://")
        || lower.starts_with("about:")
        || lower.starts_with("edge://")
        || lower.starts_with("brave://")
        || lower.starts_with("file://")
}

#[derive(Debug, Deserialize)]
struct TabEvent {
    #[serde(rename = "type")]
    kind: String,
    url: Option<String>,
    title: Option<String>,
    domain: Option<String>,
    app_name: Option<String>,
}

fn find_open_browser_event_qb() -> QueryBuilder<'static, Sqlite> {
    let mut qb = QueryBuilder::new(
        "SELECT id, app_name, browser_domain FROM activity_events \
         WHERE ended_at IS NULL AND app_name IN (",
    );
    {
        let mut sep = qb.separated(", ");
        for name in BROWSER_APPS.iter() {
            sep.push_bind(*name);
        }
    }
    qb.push(") ORDER BY started_at DESC LIMIT 1");
    qb
}

async fn apply_tab_event(app: &AppHandle, raw: &str) -> Result<(), String> {
    if raw.trim().is_empty() {
        eprintln!("[browser_bridge] skip (empty message)");
        return Ok(());
    }

    let v: Value = serde_json::from_str(raw).map_err(|e| {
        let msg = format!("json parse: {e}");
        eprintln!("[browser_bridge] ERROR {msg}");
        msg
    })?;
    if v.get("type").and_then(|x| x.as_str()) != Some("tab_event") {
        eprintln!("[browser_bridge] skip (not tab_event)");
        return Ok(());
    }
    let evt: TabEvent = serde_json::from_value(v).map_err(|e| {
        let msg = format!("tab_event deserialize: {e}");
        eprintln!("[browser_bridge] ERROR {msg}");
        msg
    })?;
    if evt.kind != "tab_event" {
        eprintln!("[browser_bridge] skip (kind != tab_event)");
        return Ok(());
    }

    let url = evt.url.unwrap_or_default();
    let title = evt.title.clone();

    if blocked_browser_scheme(&url) {
        eprintln!(
            "[browser_bridge] skip (blocked scheme) url={}",
            url.chars().take(120).collect::<String>()
        );
        return Ok(());
    }

    let domain_resolved = evt
        .domain
        .as_ref()
        .and_then(|d| {
            let t = d.trim();
            if t.is_empty() {
                None
            } else {
                Some(t.to_ascii_lowercase())
            }
        })
        .or_else(|| host_from_url(&url));

    let domain = match domain_resolved {
        Some(ref d) if !d.is_empty() => d.clone(),
        _ => {
            eprintln!(
                "[browser_bridge] skip (empty domain) url={}",
                url.chars().take(120).collect::<String>()
            );
            return Ok(());
        }
    };

    eprintln!(
        "[browser_bridge] received: domain={:?} url={}",
        Some(domain.as_str()),
        url.chars().take(200).collect::<String>()
    );

    let pool = sqlite_pool(app).await.map_err(|e| {
        eprintln!("[browser_bridge] sqlite_pool ERROR: {e}");
        e
    })?;
    let instances = app.state::<DbInstances>();

    let app_name_default = evt
        .app_name
        .as_ref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .unwrap_or("Google Chrome");

    let mut qb = find_open_browser_event_qb();
    eprintln!("[browser_bridge] open_browser_sql={}", qb.sql());
    let row = qb
        .build()
        .fetch_optional(&pool)
        .await
        .map_err(|e| {
            eprintln!("[browser_bridge] sql select open browser ERROR: {e}");
            e.to_string()
        })?;

    let now = chrono::Utc::now().timestamp_millis();

    match row {
        Some(row) => {
            let open_id: i64 = row.get("id");
            let browser_app_name: String = row.get("app_name");
            let current_domain: Option<String> = row.try_get("browser_domain").ok().flatten();

            eprintln!(
                "[browser_bridge] open_event_id=Some({open_id}), current_domain={current_domain:?}"
            );

            let app_for_lookup = evt
                .app_name
                .clone()
                .filter(|s| !s.trim().is_empty())
                .unwrap_or_else(|| browser_app_name.clone());

            let domain_matches = current_domain
                .as_ref()
                .map(|d| d.eq_ignore_ascii_case(&domain))
                .unwrap_or(false);

            if domain_matches {
                eprintln!("[browser_bridge] action=update id={open_id}");
                let cat = queries::lookup_category_for_app(
                    app_for_lookup,
                    Some(domain.clone()),
                    title.clone(),
                    &instances,
                )
                .await
                .ok()
                .flatten();

                if let Some(ref c) = cat {
                    sqlx::query(
                        "UPDATE activity_events SET browser_url = ?, window_title = ?, category_id = ?, browser_domain = ? \
                         WHERE id = ?",
                    )
                    .bind(&url)
                    .bind(title.as_deref())
                    .bind(c.id)
                    .bind(&domain)
                    .bind(open_id)
                    .execute(&pool)
                    .await
                    .map_err(|e| {
                        eprintln!("[browser_bridge] sql update ERROR: {e}");
                        e.to_string()
                    })?;
                } else {
                    sqlx::query(
                        "UPDATE activity_events SET browser_url = ?, window_title = ?, browser_domain = ? WHERE id = ?",
                    )
                    .bind(&url)
                    .bind(title.as_deref())
                    .bind(&domain)
                    .bind(open_id)
                    .execute(&pool)
                    .await
                    .map_err(|e| {
                        eprintln!("[browser_bridge] sql update ERROR: {e}");
                        e.to_string()
                    })?;
                }
            } else {
                eprintln!("[browser_bridge] action=new (close open + insert) old_id={open_id}");
                window::close_activity_event_if_open(&pool, open_id, now)
                    .await
                    .map_err(|e| {
                        eprintln!("[browser_bridge] close_event ERROR: {e}");
                        e
                    })?;

                let cat = queries::lookup_category_for_app(
                    app_for_lookup,
                    Some(domain.clone()),
                    title.clone(),
                    &instances,
                )
                .await
                .ok()
                .flatten();
                let cid: Option<i64> = cat.as_ref().map(|c| c.id);

                let res = sqlx::query(
                    "INSERT INTO activity_events \
                        (app_name, window_title, browser_url, browser_domain, \
                         category_id, started_at, ended_at, duration_seconds) \
                     VALUES (?, ?, ?, ?, ?, ?, NULL, NULL)",
                )
                .bind(&browser_app_name)
                .bind(title.as_deref())
                .bind(&url)
                .bind(&domain)
                .bind(cid)
                .bind(now)
                .execute(&pool)
                .await
                .map_err(|e| {
                    eprintln!("[browser_bridge] sql insert ERROR: {e}");
                    e.to_string()
                })?;

                let new_id = res.last_insert_rowid();
                eprintln!("[browser_bridge] inserted row id={new_id}");
                engagement::set_current_event_id(Some(new_id));
            }
        }
        None => {
            eprintln!("[browser_bridge] open_event_id=None (no open browser row)");
            eprintln!("[browser_bridge] action=create_fresh app_name={app_name_default}");

            let cat = queries::lookup_category_for_app(
                app_name_default.to_string(),
                Some(domain.clone()),
                title.clone(),
                &instances,
            )
            .await
            .ok()
            .flatten();
            let cid: Option<i64> = cat.as_ref().map(|c| c.id);

            let res = sqlx::query(
                "INSERT INTO activity_events \
                    (app_name, window_title, browser_url, browser_domain, \
                     category_id, started_at, ended_at, duration_seconds) \
                 VALUES (?, ?, ?, ?, ?, ?, NULL, NULL)",
            )
            .bind(app_name_default)
            .bind(title.as_deref())
            .bind(&url)
            .bind(&domain)
            .bind(cid)
            .bind(now)
            .execute(&pool)
            .await
            .map_err(|e| {
                eprintln!("[browser_bridge] sql insert create_fresh ERROR: {e}");
                e.to_string()
            })?;

            let new_id = res.last_insert_rowid();
            eprintln!("[browser_bridge] create_fresh id={new_id}");
            engagement::set_current_event_id(Some(new_id));
        }
    }

    LAST_MESSAGE_MS.store(
        chrono::Utc::now().timestamp_millis(),
        Ordering::Relaxed,
    );
    Ok(())
}

async fn handle_socket(app: AppHandle, stream: tokio::net::TcpStream) {
    let mut ws = match accept_async(stream).await {
        Ok(w) => w,
        Err(_) => return,
    };
    CLIENT_COUNT.fetch_add(1, Ordering::Relaxed);
    while let Some(msg) = ws.next().await {
        let Ok(msg) = msg else { break };
        if msg.is_text() {
            let text = msg.to_text().unwrap_or("");
            match apply_tab_event(&app, text).await {
                Ok(()) => {}
                Err(e) => eprintln!("[browser_bridge] apply_tab_event FAILED: {e}"),
            }
        }
    }
    CLIENT_COUNT.fetch_sub(1, Ordering::Relaxed);
}

/// Spawn the localhost WebSocket listener (idempotent).
pub fn spawn_bridge(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let listener = match TcpListener::bind("127.0.0.1:9876").await {
            Ok(l) => l,
            Err(e) => {
                eprintln!("[browser_bridge] bind 127.0.0.1:9876 failed: {e}");
                SERVER_UP.store(false, Ordering::Relaxed);
                return;
            }
        };
        SERVER_UP.store(true, Ordering::Relaxed);
        loop {
            match listener.accept().await {
                Ok((stream, _)) => {
                    let app2 = app.clone();
                    tokio::spawn(async move {
                        handle_socket(app2, stream).await;
                    });
                }
                Err(e) => eprintln!("[browser_bridge] accept: {e}"),
            }
        }
    });
}
