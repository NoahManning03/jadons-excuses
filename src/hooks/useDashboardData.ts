import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

// ---------------------------------------------------------------------------
// Shapes mirror the Rust types in
// `src-tauri/src/analytics/dashboard.rs`. If you rename a field there you
// must rename it here — there is no shared codegen.
// ---------------------------------------------------------------------------

export interface TodayOverview {
  tracked_seconds: number;
  active_seconds: number;
  idle_seconds: number;
  intense_seconds: number;
  switch_count: number;
  fragmentation_score: number;
  focus_score: number;
  engagement_sample_count: number;
}

export interface TopApp {
  app_name: string;
  total_seconds: number;
  event_count: number;
  category_id: number | null;
  category_name: string | null;
  category_color: string | null;
  active_seconds: number;
  avg_engagement: number;
}

export interface TopActivity {
  kind: string;
  name: string;
  icon_hint: string | null;
  total_seconds: number;
  active_seconds: number;
  avg_engagement: number;
  category_name: string | null;
  category_color: string | null;
}

export interface HourPoint {
  hour: number;
  avg_engagement: number;
  active_minutes: number;
}

export interface DashboardData {
  overview: TodayOverview | null;
  /** Mixed apps + websites for “Where your day went”. */
  topActivity: TopActivity[];
  hourly: HourPoint[];
  isLoading: boolean;
  /** True during every overview/top/hourly fetch (including refresh). */
  isFetching: boolean;
  error: string | null;
  /** Unix ms when overview/top/hourly were last fetched (for live counters). */
  dataFetchedAt: number;
  /** Imperative refresh from a button or after the user pauses tracking. */
  refresh: () => void;
}

/**
 * Refresh interval. 5s keeps SQLite reads light while staying fresher than
 * the old 15s default.
 */
const REFRESH_INTERVAL_MS = 5000;
const TOP_ACTIVITY_LIMIT = 10;

/**
 * Fetch and live-refresh all data the Dashboard page needs. Three Tauri
 * commands are invoked in parallel each cycle. We never throw — partial
 * results are surfaced via `error` and the previously-known data is kept
 * on screen so a transient DB hiccup doesn't blank the page.
 */
export function useDashboardData(): DashboardData {
  const [overview, setOverview] = useState<TodayOverview | null>(null);
  const [topActivity, setTopActivity] = useState<TopActivity[]>([]);
  const [hourly, setHourly] = useState<HourPoint[]>([]);
  const [isLoading, setIsLoading] = useState(true);
  const [isFetching, setIsFetching] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [dataFetchedAt, setDataFetchedAt] = useState(() => Date.now());

  const [tick, setTick] = useState(0);
  const cancelledRef = useRef(false);

  useEffect(() => {
    cancelledRef.current = false;

    const fetchAll = async () => {
      setIsFetching(true);
      try {
        const [ov, ta, hr] = await Promise.all([
          invoke<TodayOverview>("get_today_overview"),
          invoke<TopActivity[]>("get_top_activity_today", {
            limit: TOP_ACTIVITY_LIMIT,
          }),
          invoke<HourPoint[]>("get_hourly_engagement_today"),
        ]);
        if (cancelledRef.current) return;
        setOverview(ov);
        setTopActivity(ta);
        setHourly(hr);
        setDataFetchedAt(Date.now());
        setError(null);
      } catch (err) {
        if (cancelledRef.current) return;
        setError(String(err));
      } finally {
        if (!cancelledRef.current) {
          setIsLoading(false);
          setIsFetching(false);
        }
      }
    };

    fetchAll();

    let intervalId: number | null = null;
    const startPolling = () => {
      if (intervalId !== null) return;
      intervalId = window.setInterval(fetchAll, REFRESH_INTERVAL_MS);
    };
    const stopPolling = () => {
      if (intervalId !== null) {
        window.clearInterval(intervalId);
        intervalId = null;
      }
    };

    if (document.visibilityState === "visible") startPolling();

    const onVisibilityChange = () => {
      if (document.visibilityState === "visible") {
        fetchAll();
        startPolling();
      } else {
        stopPolling();
      }
    };
    document.addEventListener("visibilitychange", onVisibilityChange);

    return () => {
      cancelledRef.current = true;
      stopPolling();
      document.removeEventListener("visibilitychange", onVisibilityChange);
    };
  }, [tick]);

  return {
    overview,
    topActivity,
    hourly,
    isLoading,
    isFetching,
    error,
    dataFetchedAt,
    refresh: () => setTick((n) => n + 1),
  };
}
