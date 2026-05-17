//! ============================================================================
//! Engagement scoring engine — the core differentiator for Jadon's Excuses.
//!
//! This module measures how *engaged* the user actually is, not just whether
//! an app happens to be foregrounded. It feeds the dashboard's "active vs.
//! passive vs. idle" breakdown and is the signal the focus-session miner
//! (Step 7) keys off when deciding whether a window of focus was real work
//! or just a tab the user forgot to close.
//!
//! ============================================================================
//! PRIVACY GUARANTEES — read this before changing anything below.
//! ============================================================================
//!
//!   1. NEVER log keystroke contents.
//!      Only counts ever leave the input-polling thread. The
//!      `EngagementEvent::Key` variant is a unit value — it carries no
//!      key code, no scancode, no modifier state, nothing. Even at the
//!      Rust type level there is no way to leak which key was pressed.
//!      device_query *does* expose `Keycode` enum values via `get_keys()`,
//!      but those `Keycode`s are consumed by the polling thread *only* to
//!      compute `len(current_set - previous_set)`. They are dropped before
//!      anything is written to a channel, a log, or the DB.
//!
//!   2. NEVER capture screen contents.
//!      We do not link CoreGraphics' window-image APIs, do not call
//!      CGWindowListCreateImage, do not run AVCaptureScreenInput. Window
//!      titles are gathered by the *foreground tracker* (a sibling module)
//!      via `active-win-pos-rs`, but no pixels are ever read.
//!
//!   3. NEVER read clipboard.
//!      We do not link NSPasteboard, do not call GetClipboardData, do not
//!      poll xclip / wl-paste. There is no clipboard code path in this crate
//!      at all.
//!
//!   4. ONLY track event counts and aggregate metrics.
//!      Every value we persist to SQLite is a non-negative integer:
//!      mouse_clicks, key_presses, mouse_distance_pixels, scroll_events,
//!      is_idle (0/1), engagement_score (0–100). No payload data, ever.
//!
//! If you are tempted to add `Keycode` or any key-identifying field to
//! `EngagementEvent`, stop and revisit the privacy promise above. There is
//! no legitimate UX reason to do so — every dashboard view is built on
//! aggregate counts.
//!
//! ============================================================================
//! Architecture
//! ============================================================================
//!
//! ```text
//!   ┌─────────────────────────────┐
//!   │ device_query std::thread    │   polls every 50ms; diffs key sets,
//!   │ (NOT a Tokio task)          │   button states, and cursor position
//!   └────────────┬────────────────┘
//!                │ crossbeam-channel  (Key | Click | MouseMove(x,y))
//!                ▼
//!   ┌─────────────────────────┐
//!   │ async sampler (tokio)   │   drains channel every 100ms,
//!   │                         │   flushes to SQLite every 10s
//!   └────────────┬────────────┘
//!                │ sqlx
//!                ▼
//!         engagement_samples
//! ```
//!
//! ## Why polling (was: callback)
//!
//! This module previously used `rdev::listen()` (callback-based). rdev's
//! last release was 2023-06-26 and on recent macOS builds (Sonoma 14+ /
//! Sequoia 15+) Apple tightened the TCC rules around keyboard events from
//! global event taps. The empirical result: across 123 engagement samples
//! on the dev machine, mouse clicks / scrolls / pixel-distance all
//! populated correctly but **every key_presses field was 0** — even
//! during active typing. We swapped to `device_query` (4.0.1, released
//! 2025-07-21, actively maintained), which uses CGEventSource state
//! reads (`readkey` on macOS) instead of a session event tap and is not
//! affected by the keyboard-event-tap restriction.
//!
//! The cost of the swap is converting from event-driven to polling: we
//! tick every 50 ms, snapshot the keyboard / mouse state, and diff
//! against the previous snapshot to derive transitions. 50 ms is a
//! deliberate balance:
//!   * Fast enough to catch every keypress: human key dwell time is
//!     ~70–150 ms during typing, so a 50 ms poll always lands in-press
//!     for at least one tick (and the set-difference logic is robust to
//!     held-key repeats — the same key in two consecutive ticks counts
//!     once, not twice).
//!   * Slow enough to be free: 20 polls/sec, each is two read-only
//!     OS calls, well under any conceivable budget.
//! Going to 10 ms would 5× the CPU for zero detection benefit; going to
//! 100 ms would risk dropping fast double-keystrokes.
//!
//! ## Why a dedicated std::thread (and not a `tokio::task::spawn_blocking`)?
//!
//! Two reasons:
//!   * On macOS, device_query's `readkey` backend prefers being called
//!     from a thread with a stable identity (it caches CGEventSource
//!     handles). A dedicated thread satisfies that.
//!   * The existing scaffolding is a dedicated std::thread; reusing the
//!     shape minimizes the diff. Tokio worker threads get parked /
//!     migrated, so a 50 ms `thread::sleep` would be wasteful there.
//!
//! ## Permissions
//!
//! On macOS, the relevant TCC bucket for `device_query::DeviceState::get_keys()`
//! is **Accessibility**, not Input Monitoring. (This is a behavioural
//! change from rdev, where Input Monitoring was the gate.) The
//! distinction matters less than it sounds: `active-win-pos-rs` already
//! requires Accessibility for the window tracker, so the user has
//! almost certainly already granted it. We keep the legacy
//! `request_input_monitoring_permission` Tauri command and Settings
//! button as-is — it opens the Input Monitoring pane and the right
//! pragmatic answer is "grant *both* if asked." The detection heuristic
//! ("first received key/click event flips `HAS_INPUT_PERMISSIONS=true`")
//! also stays unchanged: it's still the only signal we get on macOS,
//! which silently returns empty key/button sets if permission is denied.
//!
//! If the polling loop panics (e.g. device_query's X11 backend on a
//! display-less Linux box), we catch it via `std::panic::catch_unwind`,
//! mark `INPUT_LISTENER_ERRORED=true`, sleep, and retry in a loop so the
//! panic never reaches the thread root. (`abort()` is not catchable.)

use std::collections::{HashSet, VecDeque};
use std::panic::AssertUnwindSafe;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicI64, AtomicU64, Ordering};
use std::sync::{Mutex as StdMutex, OnceLock};
use std::thread;
use std::time::Duration;

use chrono::Utc;
use crossbeam_channel::{unbounded, Receiver, Sender};
use device_query::{DeviceQuery, DeviceState, Keycode};
use serde::{Deserialize, Serialize};
use sqlx::{Pool, Row, Sqlite};
use tauri::{AppHandle, Manager};
use tauri_plugin_sql::{DbInstances, DbPool};

use crate::db::queries::DB_URL;

// --- constants ------------------------------------------------------------

/// `i64::MIN` is our sentinel for "no current event". Real activity_event
/// ids are positive (SQLite AUTOINCREMENT), so collisions are impossible.
const NO_EVENT: i64 = i64::MIN;

/// 10s buckets — long enough to smooth out single-keystroke spikes, short
/// enough that the timeline doesn't feel laggy in the dashboard.
const SAMPLE_INTERVAL: Duration = Duration::from_secs(10);
const SAMPLE_INTERVAL_MS: u64 = 10_000;
const SAMPLE_INTERVAL_SECS: u64 = SAMPLE_INTERVAL_MS / 1000;

/// How often the sampler drains the input-event channel into in-memory
/// counters. This is *not* the flush-to-DB cadence (that's
/// `SAMPLE_INTERVAL`). It just prevents the channel from growing
/// unbounded between flushes when the user is mashing keys.
const DRAIN_INTERVAL: Duration = Duration::from_millis(100);

/// How often the device_query polling thread reads keyboard / mouse
/// state. See the "Why polling" section in the module header for the
/// rationale on 50 ms.
const POLL_INTERVAL: Duration = Duration::from_millis(50);

/// 6 consecutive idle samples (60 s) → emit "idle_started". Tunable; this
/// matches the threshold most "are you still there?" prompts use.
const IDLE_SAMPLES_FOR_IDLE_START: u32 = 6;

/// Minimum mouse pixel-distance before we consider the user "moved" the
/// cursor. Sub-10px deltas in 10s are usually trackpad noise / inertia.
const IDLE_DISTANCE_THRESHOLD_PX: u32 = 10;

// --- atomics & shared state ----------------------------------------------

/// True between `start()` and `stop()`. The polling loop checks this on
/// every tick so we don't queue events while the engine is paused.
static RUNNING: AtomicBool = AtomicBool::new(false);

/// True while the device_query polling thread is alive. Spawned at most
/// once per process lifetime — the polling loop is cheap (50 ms tick,
/// two read-only OS calls per tick) so we leave it running across
/// `start()/stop()` cycles and just gate sends on `RUNNING`.
static LISTENER_SPAWNED: AtomicBool = AtomicBool::new(false);

/// True while the async sampler task is alive. Toggled on by `start()`,
/// off by the task itself when it observes `RUNNING == false`.
static SAMPLER_RUNNING: AtomicBool = AtomicBool::new(false);

/// True once we've observed at least one input event from device_query.
/// Stays false forever on macOS if Accessibility (the underlying TCC
/// bucket device_query reads through) is denied — that's the signal the
/// UI uses to show the "Request Input Monitoring" prompt. (The button
/// name is legacy; granting Accessibility is the operative fix.)
static HAS_INPUT_PERMISSIONS: AtomicBool = AtomicBool::new(false);

/// Set to `true` if the polling thread panicked (e.g. device_query's X11
/// backend failing on a headless Linux box). The UI uses this to decide
/// whether to show "denied" copy specifically.
static INPUT_LISTENER_ERRORED: AtomicBool = AtomicBool::new(false);

/// activity_event id the sampler will stamp onto new rows. Updated by the
/// foreground tracker on every focus change.
static CURRENT_EVENT_ID: AtomicI64 = AtomicI64::new(NO_EVENT);

/// Most recent computed score (0–100). Stored as i32 with a sentinel of -1
/// for "no sample yet". Read by `get_current_engagement` for the UI.
static LAST_SCORE: AtomicI32 = AtomicI32::new(-1);

/// Cumulative count of engagement_samples inserted today. Refreshed from
/// the DB on sampler start, then incremented locally on each insert.
static TOTAL_SAMPLES_TODAY: AtomicU64 = AtomicU64::new(0);

/// Shared crossbeam channel for ferrying input events from the polling
/// thread to the async sampler. Unbounded because the receiver drains
/// every 100 ms — far faster than the 50 ms producer can push — so the
/// queue is pretty much always empty in practice. We keep crossbeam
/// (vs std::sync::mpsc) for parity with the previous rdev-based design
/// and to keep the option of multi-producer open.
fn channel() -> &'static (Sender<EngagementEvent>, Receiver<EngagementEvent>) {
    static C: OnceLock<(Sender<EngagementEvent>, Receiver<EngagementEvent>)> = OnceLock::new();
    C.get_or_init(unbounded)
}

/// Most recent computed state label, e.g. "active". `String` because we
/// hand it to the frontend verbatim.
fn last_state_handle() -> &'static StdMutex<String> {
    static H: OnceLock<StdMutex<String>> = OnceLock::new();
    H.get_or_init(|| StdMutex::new("idle".to_string()))
}

/// Timestamps (ms since epoch) of the last 6 sample inserts. Used to
/// compute `samples_in_last_minute` for the UI. We trim eagerly so the
/// `len()` is always a valid count for the trailing 60s window.
fn last_minute_handle() -> &'static StdMutex<VecDeque<i64>> {
    static H: OnceLock<StdMutex<VecDeque<i64>>> = OnceLock::new();
    H.get_or_init(|| StdMutex::new(VecDeque::with_capacity(8)))
}

// --- types ---------------------------------------------------------------

/// The only kind of value that crosses the polling-thread → sampler
/// boundary.
///
/// Note carefully: `Key` is a unit-style variant. There is no `Key(u32)`,
/// no `Key(Keycode)`, no `Key { name: String }`. This is intentional and
/// load-bearing for the privacy guarantee. `Click` and `Scroll` are
/// similarly content-free. `MouseMove` carries x/y so the receiver can
/// compute pixel-distance deltas (the polling thread does its own diff
/// against the previous tick's position, but the receiver re-derives a
/// running-total distance for the 10 s sample bucket).
///
/// `Scroll` is preserved for ABI compat with the schema's `scroll_events`
/// column and the existing receiver match arm, but the device_query
/// backend never emits it (device_query has no wheel API). See the
/// "Scroll handling" note in `compute_score` below.
#[derive(Debug, Clone, Copy)]
pub enum EngagementEvent {
    Key,
    Click,
    /// `(x, y)` in screen pixels.
    MouseMove(f64, f64),
    Scroll,
}

/// One row of `engagement_samples`, suitable for serializing to the
/// frontend.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EngagementSample {
    pub id: i64,
    pub activity_event_id: Option<i64>,
    pub sampled_at: i64,
    pub mouse_clicks: u32,
    pub key_presses: u32,
    pub mouse_distance_pixels: u32,
    pub scroll_events: u32,
    pub is_idle: bool,
    pub engagement_score: u8,
}

/// Snapshot the Settings page polls every 2s.
#[derive(Debug, Clone, Serialize)]
pub struct CurrentEngagement {
    pub current_score: u8,
    pub current_state: String,
    pub samples_in_last_minute: u32,
    pub total_samples_today: u64,
    pub has_input_permissions: bool,
    pub listener_errored: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct HourlyEngagement {
    /// 0..=23, in the user's local-day-as-UTC bucketing. We use UTC
    /// midnight as the day boundary because the rest of the app does.
    pub hour: u8,
    pub avg_score: u8,
    pub sample_count: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct TimeInState {
    pub idle: u64,
    pub light: u64,
    pub passive: u64,
    pub active: u64,
    pub intense: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct EngagementSummary {
    pub hourly_avg: Vec<HourlyEngagement>,
    pub overall_avg: u8,
    pub time_in_state: TimeInState,
    pub total_samples: u64,
    pub active_samples: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct EventEngagement {
    pub samples: Vec<EngagementSample>,
    pub avg_score: u8,
    pub total_seconds_active: u64,
}

// --- public lifecycle ----------------------------------------------------

pub fn is_running() -> bool {
    RUNNING.load(Ordering::SeqCst)
}

pub fn has_input_permissions() -> bool {
    HAS_INPUT_PERMISSIONS.load(Ordering::SeqCst)
}

/// Spin up the engagement engine. Idempotent — calling twice while
/// running is a no-op (we use compare_exchange on `RUNNING`). Calling
/// after a `stop()` re-arms it; the device_query polling thread is
/// reused across cycles (it gates its sends on `RUNNING`).
pub fn start(app: AppHandle) {
    if RUNNING
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return; // already running
    }

    spawn_input_listener();
    spawn_sampler(app);
}

/// Stop the engine. The async sampler will exit on its next 100ms tick.
/// The polling thread keeps running but its sends are gated behind
/// `RUNNING`, so they become no-ops and the channel doesn't grow.
pub fn stop() {
    RUNNING.store(false, Ordering::SeqCst);
    CURRENT_EVENT_ID.store(NO_EVENT, Ordering::SeqCst);
}

/// Called by the foreground tracker on every focus change. `None` means
/// "no current event" (e.g. tracker just started or lost the focus).
pub fn set_current_event_id(id: Option<i64>) {
    CURRENT_EVENT_ID.store(id.unwrap_or(NO_EVENT), Ordering::SeqCst);
}

pub fn current_event_id() -> Option<i64> {
    let v = CURRENT_EVENT_ID.load(Ordering::SeqCst);
    if v == NO_EVENT {
        None
    } else {
        Some(v)
    }
}

// --- input polling thread ------------------------------------------------

fn spawn_input_listener() {
    if LISTENER_SPAWNED
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return; // already running for the lifetime of the process
    }

    let tx = channel().0.clone();
    thread::Builder::new()
        .name("jadons-engagement-input".into())
        .spawn(move || {
            loop {
                INPUT_LISTENER_ERRORED.store(false, Ordering::SeqCst);
                let result = std::panic::catch_unwind(AssertUnwindSafe(|| {
                    match DeviceState::checked_new() {
                        Some(ds) => run_polling_loop(&tx, ds),
                        None => {
                            eprintln!(
                                "[engagement] Accessibility not granted, skipping input polling"
                            );
                        }
                    }
                }));

                match result {
                    Ok(()) => break,
                    Err(_) => {
                        eprintln!("[engagement] input polling panicked, retrying in 2s");
                        HAS_INPUT_PERMISSIONS.store(false, Ordering::SeqCst);
                        INPUT_LISTENER_ERRORED.store(true, Ordering::SeqCst);
                        thread::sleep(Duration::from_secs(2));
                    }
                }
            }

            LISTENER_SPAWNED.store(false, Ordering::SeqCst);
        })
        .expect("failed to spawn engagement input listener thread");
}

/// Tight 50 ms polling loop. Reads keyboard / mouse state via
/// device_query, diffs it against the previous tick's snapshot, and
/// pushes one `EngagementEvent` per detected transition into the
/// crossbeam channel.
///
/// **Privacy.** This is the only place in the codebase where raw
/// `Keycode` values are observable. They live in `current_keys` /
/// `previous_keys` for the duration of one set-difference computation
/// and are then dropped. Nothing about *which* keys were pressed
/// crosses out of this function — only the *count* of newly-pressed
/// keys (as repeated `EngagementEvent::Key` sends), which the receiver
/// turns into a single `key_presses` integer per 10 s bucket.
fn run_polling_loop(tx: &Sender<EngagementEvent>, device_state: DeviceState) {
    let mut previous_keys: HashSet<Keycode> = HashSet::new();
    let mut previous_buttons: Vec<bool> = Vec::new();
    let mut previous_pos: Option<(i32, i32)> = None;

    loop {
        // We sleep first so a freshly spawned thread doesn't race the
        // first frame of input state on app launch.
        thread::sleep(POLL_INTERVAL);

        if !RUNNING.load(Ordering::Relaxed) {
            // Engine paused — keep the loop alive but skip work. We
            // also reset the `previous_*` snapshots so the first tick
            // after resume doesn't synthesize phantom presses for keys
            // that were already held when we paused.
            previous_keys.clear();
            previous_buttons.clear();
            previous_pos = None;
            continue;
        }

        // --- keyboard ---
        // Snapshot the current set of held keys. `keys_vec` and
        // `current_keys` are intentionally short-lived locals; the
        // `Keycode` values never escape this scope.
        let keys_vec = device_state.get_keys();
        let current_keys: HashSet<Keycode> = keys_vec.into_iter().collect();
        // New keys this tick = current - previous. We only need the
        // count, not the values.
        let new_key_count = current_keys.difference(&previous_keys).count();
        for _ in 0..new_key_count {
            // Crossbeam unbounded send only fails if the receiver was
            // dropped, which won't happen in our process model.
            let _ = tx.send(EngagementEvent::Key);
        }
        previous_keys = current_keys;

        // --- mouse buttons + position ---
        let mouse = device_state.get_mouse();
        let current_buttons: Vec<bool> = mouse.button_pressed.clone();
        // Detect newly-pressed buttons via index-wise diff. device_query
        // returns a `Vec<bool>` whose length is platform-dependent
        // (typically 5 on macOS); we tolerate any length on either side.
        let max_len = current_buttons.len().max(previous_buttons.len());
        let mut new_click_count = 0usize;
        for i in 0..max_len {
            let now = current_buttons.get(i).copied().unwrap_or(false);
            let then = previous_buttons.get(i).copied().unwrap_or(false);
            if now && !then {
                new_click_count += 1;
            }
        }
        for _ in 0..new_click_count {
            let _ = tx.send(EngagementEvent::Click);
        }
        previous_buttons = current_buttons;

        // Cursor position: only emit when it actually moved. The
        // receiver does its own running-total distance accumulation
        // against `last_pos`, so a single MouseMove per changed-tick is
        // enough.
        let (cx, cy) = mouse.coords;
        if previous_pos != Some((cx, cy)) {
            let _ = tx.send(EngagementEvent::MouseMove(cx as f64, cy as f64));
            previous_pos = Some((cx, cy));
        }
    }
}

// --- async sampler -------------------------------------------------------

fn spawn_sampler(app: AppHandle) {
    if SAMPLER_RUNNING
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return; // already running
    }
    tauri::async_runtime::spawn(async move {
        run_sampler(app).await;
        SAMPLER_RUNNING.store(false, Ordering::SeqCst);
    });
}

async fn run_sampler(app: AppHandle) {
    let Some(pool) = wait_for_pool(&app).await else {
        eprintln!("[engagement] sampler giving up — DB pool never came online");
        return;
    };

    refresh_today_count(&pool).await;

    let rx = channel().1.clone();

    // Per-bucket counters. Reset every SAMPLE_INTERVAL.
    let mut clicks: u32 = 0;
    let mut keys: u32 = 0;
    let mut distance_px: u32 = 0;
    let mut scrolls: u32 = 0;
    let mut last_pos: Option<(f64, f64)> = None;

    // Idle bookkeeping.
    let mut consecutive_idle: u32 = 0;
    let mut idle_started_at_ms: Option<i64> = None;
    let mut idle_logged: bool = false;

    let mut tick = tokio::time::interval(DRAIN_INTERVAL);
    // First tick fires immediately; skip it so our bucket boundary aligns
    // to "first tick after start".
    tick.tick().await;
    let mut elapsed = Duration::ZERO;

    loop {
        tick.tick().await;
        if !RUNNING.load(Ordering::SeqCst) {
            break;
        }

        // Drain every queued input event into in-memory counters. We hold
        // the bucket counters as locals to avoid a hot atomic spin.
        while let Ok(ev) = rx.try_recv() {
            // First event ever observed → input monitoring is granted.
            // Store-once-only via swap so we don't thrash the cache line.
            if !HAS_INPUT_PERMISSIONS.load(Ordering::Relaxed) {
                HAS_INPUT_PERMISSIONS.store(true, Ordering::SeqCst);
            }
            match ev {
                EngagementEvent::Key => {
                    keys = keys.saturating_add(1);
                }
                EngagementEvent::Click => {
                    clicks = clicks.saturating_add(1);
                }
                EngagementEvent::MouseMove(x, y) => {
                    if let Some((lx, ly)) = last_pos {
                        let dx = x - lx;
                        let dy = y - ly;
                        // sqrt is fine — even at 1000 events/sec we only do
                        // 100 sqrts per drain tick (10 ticks/sec), well
                        // under any conceivable budget.
                        let d = (dx * dx + dy * dy).sqrt();
                        if d.is_finite() && d >= 0.0 {
                            distance_px = distance_px.saturating_add(d as u32);
                        }
                    }
                    last_pos = Some((x, y));
                }
                EngagementEvent::Scroll => {
                    scrolls = scrolls.saturating_add(1);
                }
            }
        }

        elapsed += DRAIN_INTERVAL;
        if elapsed < SAMPLE_INTERVAL {
            continue;
        }
        elapsed = Duration::ZERO;

        // device_query backend never produces scroll events (see
        // compute_score doc comment), so `scrolls` is effectively
        // always 0. We still bind it to the DB column below for
        // schema-compat and future-proofing.
        let score = compute_score(clicks, keys, distance_px);
        let state = score_to_state(score);
        let now = now_ms();
        let is_idle = score == 0;

        // Surface the score/state to the UI atomically before we do any DB
        // work. The UI polling cadence (2s) is faster than our flush
        // cadence (10s), so latency here matters.
        LAST_SCORE.store(score as i32, Ordering::SeqCst);
        if let Ok(mut s) = last_state_handle().lock() {
            *s = state.to_string();
        }

        // Only persist samples that we can attach to an event. If the
        // foreground tracker hasn't seen a window yet (or the user is on a
        // login screen, etc.), we drop the sample silently. The score is
        // still updated for the UI.
        if let Some(eid) = current_event_id() {
            if let Err(e) = insert_sample(
                &pool, eid, now, clicks, keys, distance_px, scrolls, is_idle, score,
            )
            .await
            {
                eprintln!("[engagement] insert_sample failed: {e}");
            } else {
                TOTAL_SAMPLES_TODAY.fetch_add(1, Ordering::SeqCst);
                if let Ok(mut q) = last_minute_handle().lock() {
                    q.push_back(now);
                    while q.front().map(|t| now - t > 60_000).unwrap_or(false) {
                        q.pop_front();
                    }
                }
            }
        }

        // Idle transition logging. We log on the *6th* consecutive idle
        // sample (== 60s of confirmed idleness), and pair the start with an
        // end log so a downstream "screen-time" feature can compute idle
        // gaps without re-deriving them.
        if is_idle {
            consecutive_idle = consecutive_idle.saturating_add(1);
            if idle_started_at_ms.is_none() {
                idle_started_at_ms = Some(now);
            }
            if consecutive_idle >= IDLE_SAMPLES_FOR_IDLE_START && !idle_logged {
                let started = idle_started_at_ms.unwrap_or(now);
                eprintln!(
                    "[engagement] idle_started started_at_ms={started} \
                     consecutive_samples={consecutive_idle}"
                );
                idle_logged = true;
            }
        } else if idle_logged {
            let started = idle_started_at_ms.unwrap_or(now);
            let duration_s = (now - started).max(0) / 1000;
            eprintln!("[engagement] idle_ended duration_seconds={duration_s}");
            consecutive_idle = 0;
            idle_started_at_ms = None;
            idle_logged = false;
        } else if !is_idle {
            // Non-idle sample but we never crossed the 60s threshold — just
            // reset the counters quietly.
            consecutive_idle = 0;
            idle_started_at_ms = None;
        }

        // Reset bucket. Keep `last_pos` — pixel distance is per-bucket,
        // not per-stroke, but the cursor position is continuous.
        clicks = 0;
        keys = 0;
        distance_px = 0;
        scrolls = 0;
    }
}

// --- scoring -------------------------------------------------------------

/// Engagement score, 0–100. Weighted toward keyboard input — that's the
/// strongest "writing or coding" signal we have. Mouse movement alone
/// tops out at +5 because cursor drift during passive video / docs
/// reading shouldn't read as engaged.
///
/// **Scroll handling — important.** Scroll-wheel events used to
/// contribute up to +32 to the score (`(scrolls.min(40) as f32 * 0.8)`),
/// but the device_query backend has no scroll API and the previous
/// rdev backend was dropped because of a macOS keyboard regression.
/// Rather than rebuild a separate event-tap dependency just for wheel,
/// scrolls are dropped from the formula entirely. The DB column
/// `engagement_samples.scroll_events` is preserved (always 0 today) so
/// a future scroll-capable backend can backfill it without a schema
/// migration.
///
/// Formula:
///   * Idle iff `clicks == 0 && keys == 0 && distance_px < 10`.
///   * Else: `keys.min(50) * 1.6  +  clicks.min(30) * 1.2  +  (5 if distance_px > 100)`,
///     capped at 100.
///
/// Maximums:
///   * Keys alone: 80  → "active"
///   * Clicks alone: 36 → "passive"
///   * Distance alone: 5 → "light"
///   * All three: 121 → capped at 100 → "intense"
///
/// The signature changed in the device_query swap: the trailing
/// `scrolls: u32` argument was removed. If you reintroduce a scroll
/// signal, prefer adding it as a fourth argument again rather than
/// silently changing weights — the docs in `Insights.tsx` reference
/// these numbers.
pub fn compute_score(clicks: u32, keys: u32, distance_px: u32) -> u8 {
    if clicks == 0 && keys == 0 && distance_px < IDLE_DISTANCE_THRESHOLD_PX {
        return 0; // Idle
    }
    let mut score: u32 = 0;
    score += (keys.min(50) as f32 * 1.6) as u32;
    score += (clicks.min(30) as f32 * 1.2) as u32;
    if distance_px > 100 {
        score += 5;
    }
    score.min(100) as u8
}

pub fn score_to_state(s: u8) -> &'static str {
    match s {
        0 => "idle",
        1..=25 => "light",
        26..=50 => "passive",
        51..=80 => "active",
        _ => "intense",
    }
}

// --- DB helpers ----------------------------------------------------------

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

async fn wait_for_pool(app: &AppHandle) -> Option<Pool<Sqlite>> {
    for _ in 0..40 {
        if let Ok(pool) = sqlite_pool(app).await {
            return Some(pool);
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    None
}

async fn refresh_today_count(pool: &Pool<Sqlite>) {
    let start_of_today = start_of_today_utc_ms();
    let res: Result<(i64,), _> =
        sqlx::query_as("SELECT COUNT(*) FROM engagement_samples WHERE sampled_at >= ?")
            .bind(start_of_today)
            .fetch_one(pool)
            .await;
    if let Ok((c,)) = res {
        TOTAL_SAMPLES_TODAY.store(c.max(0) as u64, Ordering::SeqCst);
    }
}

#[allow(clippy::too_many_arguments)]
async fn insert_sample(
    pool: &Pool<Sqlite>,
    activity_event_id: i64,
    sampled_at: i64,
    mouse_clicks: u32,
    key_presses: u32,
    mouse_distance_pixels: u32,
    scroll_events: u32,
    is_idle: bool,
    engagement_score: u8,
) -> Result<(), String> {
    sqlx::query(
        "INSERT INTO engagement_samples \
            (activity_event_id, sampled_at, mouse_clicks, key_presses, \
             mouse_distance_pixels, scroll_events, is_idle, engagement_score) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(activity_event_id)
    .bind(sampled_at)
    .bind(mouse_clicks as i64)
    .bind(key_presses as i64)
    .bind(mouse_distance_pixels as i64)
    .bind(scroll_events as i64)
    .bind(if is_idle { 1_i64 } else { 0_i64 })
    .bind(engagement_score as i64)
    .execute(pool)
    .await
    .map(|_| ())
    .map_err(|e| e.to_string())
}

fn start_of_today_utc_ms() -> i64 {
    let now = Utc::now();
    let midnight = now.date_naive().and_hms_opt(0, 0, 0).unwrap().and_utc();
    midnight.timestamp_millis()
}

fn now_ms() -> i64 {
    Utc::now().timestamp_millis()
}

// --- Tauri-facing query API ----------------------------------------------

pub async fn get_current_engagement(app: &AppHandle) -> CurrentEngagement {
    // Refresh today count opportunistically so the panel doesn't drift
    // after a day rollover or if the user just opened the page.
    if let Ok(pool) = sqlite_pool(app).await {
        refresh_today_count(&pool).await;
    }

    let raw = LAST_SCORE.load(Ordering::SeqCst);
    let score: u8 = if raw < 0 { 0 } else { (raw.min(100)) as u8 };
    let state = last_state_handle()
        .lock()
        .map(|s| s.clone())
        .unwrap_or_else(|_| "idle".to_string());

    let samples_in_last_minute = {
        let now = now_ms();
        if let Ok(mut q) = last_minute_handle().lock() {
            while q.front().map(|t| now - t > 60_000).unwrap_or(false) {
                q.pop_front();
            }
            q.len() as u32
        } else {
            0
        }
    };

    CurrentEngagement {
        current_score: score,
        current_state: state,
        samples_in_last_minute,
        total_samples_today: TOTAL_SAMPLES_TODAY.load(Ordering::SeqCst),
        has_input_permissions: HAS_INPUT_PERMISSIONS.load(Ordering::SeqCst),
        listener_errored: INPUT_LISTENER_ERRORED.load(Ordering::SeqCst),
    }
}

/// Single-line status for the tray menu (no DB / async).
pub fn tray_status_label() -> String {
    let raw = LAST_SCORE.load(Ordering::SeqCst);
    let score: u8 = if raw < 0 { 0 } else { (raw.min(100)) as u8 };
    let state = last_state_handle()
        .lock()
        .map(|s| s.clone())
        .unwrap_or_else(|_| "idle".to_string());
    let label = match state.as_str() {
        "idle" => "Idle",
        "light" => "Light",
        "passive" => "Passive",
        "active" => "Active",
        "intense" => "Engaged",
        _ => "Idle",
    };
    format!("● {label} | Score: {score}")
}

pub fn listener_errored() -> bool {
    INPUT_LISTENER_ERRORED.load(Ordering::Relaxed)
}

pub fn has_input_permissions_flag() -> bool {
    HAS_INPUT_PERMISSIONS.load(Ordering::Relaxed)
}

pub fn last_computed_score_raw() -> i32 {
    LAST_SCORE.load(Ordering::SeqCst)
}

pub fn last_state_label() -> String {
    last_state_handle()
        .lock()
        .map(|s| s.clone())
        .unwrap_or_else(|_| "idle".to_string())
}

pub async fn get_engagement_for_today(app: &AppHandle) -> Result<EngagementSummary, String> {
    let pool = sqlite_pool(app).await?;
    let start = start_of_today_utc_ms();

    // Hourly average.
    let hourly_rows = sqlx::query(
        "SELECT CAST((sampled_at - ?) / 3600000 AS INTEGER) AS hour, \
                AVG(engagement_score) AS avg_score, \
                COUNT(*) AS cnt \
         FROM engagement_samples \
         WHERE sampled_at >= ? \
         GROUP BY hour \
         ORDER BY hour",
    )
    .bind(start)
    .bind(start)
    .fetch_all(&pool)
    .await
    .map_err(|e| e.to_string())?;

    let mut hourly: Vec<HourlyEngagement> = (0..24u8)
        .map(|h| HourlyEngagement {
            hour: h,
            avg_score: 0,
            sample_count: 0,
        })
        .collect();
    for r in hourly_rows {
        let hour: i64 = r.get("hour");
        let avg: Option<f64> = r.get("avg_score");
        let cnt: i64 = r.get("cnt");
        if (0..24).contains(&hour) {
            let idx = hour as usize;
            hourly[idx].avg_score = avg.unwrap_or(0.0).round().clamp(0.0, 100.0) as u8;
            hourly[idx].sample_count = cnt.max(0) as u64;
        }
    }

    // Overall average + total count.
    let overall_row = sqlx::query(
        "SELECT AVG(engagement_score) AS avg_s, COUNT(*) AS cnt \
         FROM engagement_samples WHERE sampled_at >= ?",
    )
    .bind(start)
    .fetch_one(&pool)
    .await
    .map_err(|e| e.to_string())?;
    let overall_avg_f: Option<f64> = overall_row.get("avg_s");
    let total_samples: i64 = overall_row.get("cnt");

    // Active count (score > 0).
    let active_row = sqlx::query(
        "SELECT COUNT(*) AS cnt FROM engagement_samples \
         WHERE sampled_at >= ? AND engagement_score > 0",
    )
    .bind(start)
    .fetch_one(&pool)
    .await
    .map_err(|e| e.to_string())?;
    let active_samples: i64 = active_row.get("cnt");

    // Time in each state. Each sample represents `SAMPLE_INTERVAL_SECS`
    // seconds of wall-clock; multiplying the bucket count gives us
    // seconds-spent-in-state. There's a small over-count if the user
    // closed/sleep'd the app mid-bucket, but it's bounded at ±10s.
    let buckets = sqlx::query(
        "SELECT \
           SUM(CASE WHEN engagement_score = 0 THEN 1 ELSE 0 END) AS idle_c, \
           SUM(CASE WHEN engagement_score BETWEEN 1 AND 25 THEN 1 ELSE 0 END) AS light_c, \
           SUM(CASE WHEN engagement_score BETWEEN 26 AND 50 THEN 1 ELSE 0 END) AS passive_c, \
           SUM(CASE WHEN engagement_score BETWEEN 51 AND 80 THEN 1 ELSE 0 END) AS active_c, \
           SUM(CASE WHEN engagement_score >= 81 THEN 1 ELSE 0 END) AS intense_c \
         FROM engagement_samples WHERE sampled_at >= ?",
    )
    .bind(start)
    .fetch_one(&pool)
    .await
    .map_err(|e| e.to_string())?;

    let to_secs = |opt: Option<i64>| -> u64 {
        (opt.unwrap_or(0).max(0) as u64).saturating_mul(SAMPLE_INTERVAL_SECS)
    };

    let time_in_state = TimeInState {
        idle: to_secs(buckets.get("idle_c")),
        light: to_secs(buckets.get("light_c")),
        passive: to_secs(buckets.get("passive_c")),
        active: to_secs(buckets.get("active_c")),
        intense: to_secs(buckets.get("intense_c")),
    };

    Ok(EngagementSummary {
        hourly_avg: hourly,
        overall_avg: overall_avg_f.unwrap_or(0.0).round().clamp(0.0, 100.0) as u8,
        time_in_state,
        total_samples: total_samples.max(0) as u64,
        active_samples: active_samples.max(0) as u64,
    })
}

pub async fn get_engagement_for_event(
    app: &AppHandle,
    event_id: i64,
) -> Result<EventEngagement, String> {
    let pool = sqlite_pool(app).await?;
    let rows = sqlx::query(
        "SELECT id, activity_event_id, sampled_at, mouse_clicks, key_presses, \
                mouse_distance_pixels, scroll_events, is_idle, engagement_score \
         FROM engagement_samples \
         WHERE activity_event_id = ? \
         ORDER BY sampled_at",
    )
    .bind(event_id)
    .fetch_all(&pool)
    .await
    .map_err(|e| e.to_string())?;

    let samples: Vec<EngagementSample> = rows
        .iter()
        .map(|r| {
            let mouse_clicks: i64 = r.get("mouse_clicks");
            let key_presses: i64 = r.get("key_presses");
            let mouse_distance_pixels: i64 = r.get("mouse_distance_pixels");
            let scroll_events: i64 = r.get("scroll_events");
            let is_idle: i64 = r.get("is_idle");
            let engagement_score: i64 = r.get("engagement_score");
            EngagementSample {
                id: r.get("id"),
                activity_event_id: r.get("activity_event_id"),
                sampled_at: r.get("sampled_at"),
                mouse_clicks: mouse_clicks.max(0) as u32,
                key_presses: key_presses.max(0) as u32,
                mouse_distance_pixels: mouse_distance_pixels.max(0) as u32,
                scroll_events: scroll_events.max(0) as u32,
                is_idle: is_idle != 0,
                engagement_score: engagement_score.clamp(0, 100) as u8,
            }
        })
        .collect();

    let total_samples = samples.len() as u64;
    let active_count = samples.iter().filter(|s| !s.is_idle).count() as u64;
    let total_seconds_active = active_count.saturating_mul(SAMPLE_INTERVAL_SECS);
    let avg_score: u8 = if total_samples == 0 {
        0
    } else {
        let sum: u32 = samples.iter().map(|s| s.engagement_score as u32).sum();
        ((sum as f64) / (total_samples as f64))
            .round()
            .clamp(0.0, 100.0) as u8
    };

    Ok(EventEngagement {
        samples,
        avg_score,
        total_seconds_active,
    })
}

// --- macOS permissions helper --------------------------------------------

/// Open System Settings → Privacy & Security → Input Monitoring on macOS.
/// On other platforms this is a no-op (rdev doesn't gate input on
/// permission outside macOS — Linux/Windows either work or hard-error).
pub fn open_input_monitoring_pane() {
    #[cfg(target_os = "macos")]
    {
        // `Privacy_ListenEvent` is the legacy URL fragment for the Input
        // Monitoring pane. macOS Sonoma+ still routes it correctly even
        // though the surface UI was renamed.
        let _ = std::process::Command::new("open")
            .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_ListenEvent")
            .spawn();
    }
    #[cfg(not(target_os = "macos"))]
    {
        eprintln!("[engagement] open_input_monitoring_pane is a no-op outside macOS");
    }
}

// --- tests ---------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idle_score_is_zero() {
        assert_eq!(compute_score(0, 0, 0), 0);
        assert_eq!(compute_score(0, 0, 9), 0); // sub-threshold mouse
    }

    #[test]
    fn keys_dominate_score() {
        // 50 keys * 1.6 = 80 → "active"
        let s = compute_score(0, 50, 0);
        assert_eq!(s, 80);
        assert_eq!(score_to_state(s), "active");
    }

    #[test]
    fn caps_at_100() {
        // Was previously `compute_score(100, 100, 5000, 100)` — adapted
        // for the new 3-arg signature after scrolls were dropped from
        // the formula. Maximum reachable contribution is now
        // 80 (keys) + 36 (clicks) + 5 (distance) = 121, which still
        // saturates at 100.
        let s = compute_score(100, 100, 5000);
        assert_eq!(s, 100);
        assert_eq!(score_to_state(s), "intense");
    }

    #[test]
    fn small_movement_only_is_light() {
        // 200px of movement alone → +5
        let s = compute_score(0, 0, 200);
        assert_eq!(s, 5);
        assert_eq!(score_to_state(s), "light");
    }

    #[test]
    fn state_boundaries() {
        assert_eq!(score_to_state(0), "idle");
        assert_eq!(score_to_state(1), "light");
        assert_eq!(score_to_state(25), "light");
        assert_eq!(score_to_state(26), "passive");
        assert_eq!(score_to_state(50), "passive");
        assert_eq!(score_to_state(51), "active");
        assert_eq!(score_to_state(80), "active");
        assert_eq!(score_to_state(81), "intense");
        assert_eq!(score_to_state(100), "intense");
    }

    /// Pins the post-device_query 3-arg formula so a future contributor
    /// can't silently re-add scroll without breaking this test.
    /// Mirrors the math in the doc comment on `compute_score`.
    #[test]
    fn three_arg_formula_matches_spec() {
        // 30 keys → 30*1.6 = 48 → "passive"
        assert_eq!(compute_score(0, 30, 0), 48);
        // 30 clicks → 30*1.2 = 36 → "passive"
        assert_eq!(compute_score(30, 0, 0), 36);
        // 10 keys + 10 clicks → 16 + 12 = 28 → "passive"
        assert_eq!(compute_score(10, 10, 0), 28);
        // 10 keys + 10 clicks + 150 px (>100 → +5) = 33 → "passive"
        assert_eq!(compute_score(10, 10, 150), 33);
        // Cap test: keys+clicks at their cap + huge distance.
        assert_eq!(compute_score(30, 50, 99_999), 100);
        // Distance must be > 100, not >= 100, to score the +5.
        assert_eq!(compute_score(0, 0, 100), 0); // <10 path is false, but +5 path also false → 0
        assert_eq!(compute_score(0, 0, 101), 5);
        // Privacy invariant — function takes counts only, no key
        // identity argument exists at the type level.
        let _: u8 = compute_score(1, 1, 1);
    }
}
