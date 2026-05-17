import { useEffect, useState } from "react";
import { Link } from "react-router-dom";
import { invoke } from "@tauri-apps/api/core";
import { PagePlaceholder } from "../components/PagePlaceholder";

interface DbHealth {
  db_path: string;
  total_activity_events: number;
  total_categories: number;
  total_app_mappings: number;
  schema_version: number;
}

interface TrackingStatus {
  running: boolean;
  current_app: string | null;
  current_window_title: string | null;
  has_permissions: boolean;
  total_events_today: number;
}

interface CurrentEngagement {
  current_score: number;
  current_state: string;
  samples_in_last_minute: number;
  total_samples_today: number;
  has_input_permissions: boolean;
  listener_errored: boolean;
}

type EngagementState = "idle" | "light" | "passive" | "active" | "intense";

interface BridgeStatus {
  running: boolean;
  connected_clients: number;
  last_message_at: number | null;
}

function BridgePanel() {
  const [st, setSt] = useState<BridgeStatus | null>(null);
  useEffect(() => {
    let cancelled = false;
    const tick = () => {
      invoke<BridgeStatus>("get_bridge_status")
        .then((r) => {
          if (!cancelled) setSt(r);
        })
        .catch(() => {});
    };
    tick();
    const id = window.setInterval(tick, 2000);
    return () => {
      cancelled = true;
      window.clearInterval(id);
    };
  }, []);

  return (
    <div className="rounded-2xl border border-slate-200 bg-white p-6 shadow-soft dark:border-slate-800 dark:bg-slate-900">
      <p className="text-xs font-medium uppercase tracking-[0.18em] text-slate-500">
        Browser bridge
      </p>
      <p className="mt-2 text-sm text-slate-600 dark:text-slate-400">
        Install the unpacked extension in{" "}
        <code className="rounded bg-slate-100 px-1 dark:bg-slate-800">
          browser-extension/
        </code>{" "}
        (see{" "}
        <code className="rounded bg-slate-100 px-1 dark:bg-slate-800">
          README.md
        </code>{" "}
        there). It connects to{" "}
        <code className="rounded bg-slate-100 px-1 dark:bg-slate-800">
          ws://127.0.0.1:9876
        </code>{" "}
        while this app is running.
      </p>
      <dl className="mt-4 space-y-2 text-sm">
        <div className="flex justify-between gap-4">
          <dt className="text-slate-500">Listener</dt>
          <dd className="font-mono text-slate-900 dark:text-slate-100">
            {st?.running ? "up" : "down"}
          </dd>
        </div>
        <div className="flex justify-between gap-4">
          <dt className="text-slate-500">Extension sockets</dt>
          <dd className="font-mono text-slate-900 dark:text-slate-100">
            {st?.connected_clients ?? 0}
          </dd>
        </div>
        <div className="flex justify-between gap-4">
          <dt className="text-slate-500">Last message</dt>
          <dd className="font-mono text-xs text-slate-900 dark:text-slate-100">
            {st?.last_message_at
              ? new Date(st.last_message_at).toLocaleString()
              : "—"}
          </dd>
        </div>
      </dl>
    </div>
  );
}

export function AdvancedSettings() {
  return (
    <div className="min-h-full bg-gradient-to-b from-white to-tangerine-50/30 px-10 py-10 dark:from-slate-900 dark:to-tangerine-900/20">
      <div className="mx-auto max-w-3xl space-y-6 pb-16">
        <Link
          to="/settings"
          className="inline-flex text-sm font-medium text-tangerine-600 hover:text-tangerine-700 dark:text-tangerine-400"
        >
          ← Back to Settings
        </Link>
        <PagePlaceholder
          eyebrow="Advanced"
          title="Developer tools."
          subtitle="Database health, engagement debug, and bridge diagnostics."
        />
        <DbHealthPanel />
        <BridgePanel />
        <EngagementPanel />
      </div>
    </div>
  );
}

function DbHealthPanel() {
  const [health, setHealth] = useState<DbHealth | null>(null);
  const [healthError, setHealthError] = useState<string | null>(null);
  const [status, setStatus] = useState<TrackingStatus | null>(null);
  const [statusError, setStatusError] = useState<string | null>(null);
  const [requesting, setRequesting] = useState(false);

  useEffect(() => {
    let cancelled = false;
    invoke<DbHealth>("db_health_check")
      .then((res) => {
        if (!cancelled) setHealth(res);
      })
      .catch((err) => {
        if (!cancelled) setHealthError(String(err));
      });
    return () => {
      cancelled = true;
    };
  }, []);

  useEffect(() => {
    let cancelled = false;
    const tick = () => {
      invoke<TrackingStatus>("get_tracking_status")
        .then((res) => {
          if (!cancelled) {
            setStatus(res);
            setStatusError(null);
          }
        })
        .catch((err) => {
          if (!cancelled) setStatusError(String(err));
        });
    };
    tick();
    const id = window.setInterval(tick, 2000);
    return () => {
      cancelled = true;
      window.clearInterval(id);
    };
  }, []);

  const onRequestPermission = async () => {
    setRequesting(true);
    try {
      await invoke("request_accessibility_permission");
    } catch (err) {
      setStatusError(String(err));
    } finally {
      setRequesting(false);
    }
  };

  return (
    <div className="rounded-2xl border border-slate-200 bg-white p-6 shadow-soft dark:border-slate-800 dark:bg-slate-900">
      <div className="mb-3 flex items-center justify-between">
        <p className="text-xs font-medium uppercase tracking-[0.18em] text-slate-500">
          DB Health · debug
        </p>
        <span className="rounded-full bg-amber-100 px-2 py-0.5 text-[10px] font-medium uppercase tracking-wide text-amber-700 dark:bg-amber-900/40 dark:text-amber-300">
          temporary
        </span>
      </div>
      <p className="mb-4 text-sm text-slate-600 dark:text-slate-400">
        Verifies the SQLite migration ran, seed data is present, and the
        foreground-window tracker is alive.
      </p>

      <TrackingPanel
        status={status}
        error={statusError}
        requesting={requesting}
        onRequestPermission={onRequestPermission}
      />

      <div className="mt-6">
        <p className="mb-2 text-xs font-medium uppercase tracking-[0.18em] text-slate-500">
          Database
        </p>
        {healthError && (
          <pre className="overflow-x-auto rounded-lg bg-red-50 p-3 text-xs text-red-700 dark:bg-red-950/40 dark:text-red-300">
            {healthError}
          </pre>
        )}
        {!healthError && !health && (
          <p className="text-sm text-slate-400">Loading…</p>
        )}
        {health && (
          <pre className="overflow-x-auto rounded-lg bg-slate-50 p-4 text-xs leading-relaxed text-slate-800 dark:bg-slate-800 dark:text-slate-200">
            {JSON.stringify(health, null, 2)}
          </pre>
        )}
      </div>
    </div>
  );
}

interface TrackingPanelProps {
  status: TrackingStatus | null;
  error: string | null;
  requesting: boolean;
  onRequestPermission: () => void;
}

function TrackingPanel({
  status,
  error,
  requesting,
  onRequestPermission,
}: TrackingPanelProps) {
  return (
    <div className="rounded-xl border border-slate-200 bg-slate-50 p-4 dark:border-slate-700 dark:bg-slate-800/60">
      <p className="mb-3 text-xs font-medium uppercase tracking-[0.18em] text-slate-500">
        Tracker
      </p>
      {error && (
        <pre className="mb-3 overflow-x-auto rounded-lg bg-red-50 p-3 text-xs text-red-700 dark:bg-red-950/40 dark:text-red-300">
          {error}
        </pre>
      )}
      {!status && !error && <p className="text-sm text-slate-400">Loading…</p>}
      {status && (
        <dl className="grid grid-cols-1 gap-y-2 text-sm sm:grid-cols-2">
          <StatusRow
            label="Tracking running"
            value={status.running ? "yes" : "no"}
            tone={status.running ? "good" : "warn"}
          />
          <StatusRow
            label="Has permissions"
            value={status.has_permissions ? "yes" : "no"}
            tone={status.has_permissions ? "good" : "warn"}
          />
          <StatusRow
            label="Current app"
            value={status.current_app ?? "—"}
            tone="neutral"
          />
          <StatusRow
            label="Total events today"
            value={String(status.total_events_today)}
            tone="neutral"
          />
          {status.current_window_title && (
            <div className="sm:col-span-2">
              <dt className="text-xs uppercase tracking-wide text-slate-500">
                Current window
              </dt>
              <dd className="truncate text-sm text-slate-800 dark:text-slate-200">
                {status.current_window_title}
              </dd>
            </div>
          )}
        </dl>
      )}

      {status && !status.has_permissions && (
        <div className="mt-4 rounded-lg border border-amber-200 bg-amber-50 p-3 dark:border-amber-900/50 dark:bg-amber-950/30">
          <p className="text-sm text-amber-900 dark:text-amber-200">
            Jadon's Excuses needs Accessibility permission to know which app
            you're focused on. Without it, no activity is recorded.
          </p>
          <ol className="mt-2 list-decimal space-y-1 pl-5 text-xs text-amber-900 dark:text-amber-200">
            <li>
              Click the button below to open System Settings → Privacy &amp;
              Security → Accessibility.
            </li>
            <li>
              Click <span className="font-mono">+</span> → find{" "}
              <span className="font-medium">Jadon's Excuses</span> → toggle it
              on.
            </li>
            <li>Quit and relaunch this app.</li>
          </ol>
          <button
            type="button"
            onClick={onRequestPermission}
            disabled={requesting}
            className="mt-3 rounded-lg bg-amber-600 px-3 py-1.5 text-xs font-medium text-white shadow-sm transition hover:bg-amber-700 disabled:opacity-60"
          >
            {requesting ? "Opening…" : "Request Accessibility Permission"}
          </button>
        </div>
      )}
    </div>
  );
}

function StatusRow({
  label,
  value,
  tone,
}: {
  label: string;
  value: string;
  tone: "good" | "warn" | "neutral";
}) {
  const dotClass =
    tone === "good"
      ? "bg-emerald-500"
      : tone === "warn"
        ? "bg-amber-500"
        : "bg-slate-400";
  return (
    <div className="flex items-center gap-2">
      <span className={`h-2 w-2 rounded-full ${dotClass}`} />
      <dt className="text-xs uppercase tracking-wide text-slate-500">
        {label}
      </dt>
      <dd className="ml-auto truncate text-sm text-slate-800 dark:text-slate-200">
        {value}
      </dd>
    </div>
  );
}

const STATE_COLOR: Record<EngagementState, { dot: string; bar: string }> = {
  idle: { dot: "bg-slate-400", bar: "bg-slate-400" },
  light: { dot: "bg-blue-500", bar: "bg-blue-500" },
  passive: { dot: "bg-amber-400", bar: "bg-amber-400" },
  active: { dot: "bg-emerald-500", bar: "bg-emerald-500" },
  intense: { dot: "bg-tangerine-500", bar: "bg-tangerine-500" },
};

function asEngagementState(s: string): EngagementState {
  return (
    ["idle", "light", "passive", "active", "intense"] as const
  ).find((v) => v === s) ?? "idle";
}

function EngagementPanel() {
  const [eng, setEng] = useState<CurrentEngagement | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [requesting, setRequesting] = useState(false);

  useEffect(() => {
    let cancelled = false;
    const tick = () => {
      invoke<CurrentEngagement>("get_current_engagement")
        .then((res) => {
          if (!cancelled) {
            setEng(res);
            setError(null);
          }
        })
        .catch((err) => {
          if (!cancelled) setError(String(err));
        });
    };
    tick();
    const id = window.setInterval(tick, 2000);
    return () => {
      cancelled = true;
      window.clearInterval(id);
    };
  }, []);

  const onRequestPermission = async () => {
    setRequesting(true);
    try {
      await invoke("request_input_monitoring_permission");
    } catch (err) {
      setError(String(err));
    } finally {
      setRequesting(false);
    }
  };

  const state: EngagementState = asEngagementState(eng?.current_state ?? "idle");
  const score = eng?.current_score ?? 0;
  const colors = STATE_COLOR[state];

  return (
    <div className="rounded-2xl border border-slate-200 bg-white p-6 shadow-soft dark:border-slate-800 dark:bg-slate-900">
      <div className="mb-3 flex items-center justify-between">
        <p className="text-xs font-medium uppercase tracking-[0.18em] text-slate-500">
          Engagement · debug
        </p>
        <span className="rounded-full bg-amber-100 px-2 py-0.5 text-[10px] font-medium uppercase tracking-wide text-amber-700 dark:bg-amber-900/40 dark:text-amber-300">
          temporary
        </span>
      </div>
      <p className="mb-4 text-sm text-slate-600 dark:text-slate-400">
        Live view of the engagement scoring engine. Type or click around and
        the score + state should react within ~10 s (one bucket).
      </p>

      {error && (
        <pre className="mb-3 overflow-x-auto rounded-lg bg-red-50 p-3 text-xs text-red-700 dark:bg-red-950/40 dark:text-red-300">
          {error}
        </pre>
      )}

      <div className="rounded-xl border border-slate-200 bg-slate-50 p-4 dark:border-slate-700 dark:bg-slate-800/60">
        {!eng && !error && <p className="text-sm text-slate-400">Loading…</p>}
        {eng && (
          <>
            <div className="flex items-center gap-3">
              <span
                aria-hidden
                className={`inline-block h-3 w-3 shrink-0 rounded-full ${colors.dot}`}
              />
              <div className="flex flex-1 items-baseline gap-2">
                <span className="font-mono text-sm font-medium uppercase tracking-wide text-slate-800 dark:text-slate-200">
                  {state}
                </span>
                <span className="text-xs text-slate-500">
                  · score {score}/100
                </span>
              </div>
              <span className="text-xs text-slate-500">
                {eng.samples_in_last_minute} samples / 60 s
              </span>
            </div>

            <div className="mt-3 h-2 w-full overflow-hidden rounded-full bg-slate-200 dark:bg-slate-700">
              <div
                className={`h-full ${colors.bar} transition-all duration-500 ease-out`}
                style={{ width: `${Math.min(100, Math.max(0, score))}%` }}
              />
            </div>

            <dl className="mt-4 grid grid-cols-1 gap-y-2 text-sm sm:grid-cols-2">
              <StatusRow
                label="Total samples today"
                value={String(eng.total_samples_today)}
                tone="neutral"
              />
              <StatusRow
                label="Input monitoring"
                value={
                  eng.has_input_permissions
                    ? "granted"
                    : eng.listener_errored
                      ? "denied"
                      : "unknown"
                }
                tone={eng.has_input_permissions ? "good" : "warn"}
              />
            </dl>
          </>
        )}

        {eng && !eng.has_input_permissions && (
          <div className="mt-4 rounded-lg border border-amber-200 bg-amber-50 p-3 dark:border-amber-900/50 dark:bg-amber-950/30">
            <p className="text-sm text-amber-900 dark:text-amber-200">
              Jadon's Excuses needs <strong>Input Monitoring</strong>{" "}
              permission to count keypresses, clicks, and scrolls.
            </p>
            <button
              type="button"
              onClick={onRequestPermission}
              disabled={requesting}
              className="mt-3 rounded-lg bg-amber-600 px-3 py-1.5 text-xs font-medium text-white shadow-sm transition hover:bg-amber-700 disabled:opacity-60"
            >
              {requesting ? "Opening…" : "Request Input Monitoring Permission"}
            </button>
          </div>
        )}
      </div>
    </div>
  );
}
