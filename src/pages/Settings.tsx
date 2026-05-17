import {
  useCallback,
  useEffect,
  useState,
  type ComponentType,
  type ReactNode,
} from "react";
import { Link } from "react-router-dom";
import { invoke } from "@tauri-apps/api/core";
import { motion } from "framer-motion";
import {
  Activity,
  Bell,
  CheckCircle,
  Globe,
  Info,
  Palette,
  ShieldCheck,
  XCircle,
} from "lucide-react";
import { cn } from "../lib/utils";
import { useTheme, type ThemePreference } from "../contexts/ThemeProvider";


interface TrackingStatus {
  running: boolean;
  current_app: string | null;
  current_window_title: string | null;
  has_permissions: boolean;
  total_events_today: number;
}

interface BridgeStatus {
  running: boolean;
  connected_clients: number;
  last_message_at: number | null;
}

function SectionCard({
  icon: Icon,
  eyebrow,
  title,
  children,
}: {
  icon: ComponentType<{ className?: string }>;
  eyebrow: string;
  title: string;
  children: ReactNode;
}) {
  return (
    <section className="rounded-2xl border border-slate-100 bg-white p-6 shadow-soft dark:border-slate-800 dark:bg-slate-900">
      <div className="flex gap-4">
        <div className="flex h-11 w-11 shrink-0 items-center justify-center rounded-xl bg-tangerine-50 text-tangerine-600 dark:bg-tangerine-950/40 dark:text-tangerine-400">
          <Icon className="h-5 w-5" />
        </div>
        <div className="min-w-0 flex-1">
          <p className="text-[10px] font-semibold uppercase tracking-[0.2em] text-slate-500 dark:text-slate-400">
            {eyebrow}
          </p>
          <h2 className="mt-1 text-lg font-semibold text-slate-900 dark:text-slate-100">
            {title}
          </h2>
          <div className="mt-4">{children}</div>
        </div>
      </div>
    </section>
  );
}

function Switch({
  checked,
  onChange,
  disabled,
}: {
  checked: boolean;
  onChange: (v: boolean) => void;
  disabled?: boolean;
}) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      disabled={disabled}
      onClick={() => onChange(!checked)}
      className={cn(
        "relative inline-flex h-7 w-12 shrink-0 cursor-pointer rounded-full border-2 border-transparent transition-colors focus:outline-none focus:ring-2 focus:ring-tangerine-400 focus:ring-offset-2 disabled:cursor-not-allowed disabled:opacity-50 dark:focus:ring-offset-slate-900",
        checked ? "bg-tangerine-500" : "bg-slate-200 dark:bg-slate-700",
      )}
    >
      <span
        className={cn(
          "pointer-events-none inline-block h-6 w-6 translate-x-0.5 rounded-full bg-white shadow ring-0 transition duration-200 ease-in-out",
          checked && "translate-x-[1.35rem]",
        )}
      />
    </button>
  );
}

function bridgeUiStatus(st: BridgeStatus | null): {
  dot: string;
  label: string;
} {
  if (!st?.running) {
    return { dot: "bg-red-500", label: "Bridge offline" };
  }
  const now = Date.now();
  const fresh =
    st.connected_clients > 0 &&
    st.last_message_at != null &&
    now - st.last_message_at < 60_000;
  if (fresh) {
    return {
      dot: "bg-emerald-500",
      label: "Connected — receiving tab activity",
    };
  }
  return {
    dot: "bg-amber-400",
    label: "Bridge ready — install the browser extension to start",
  };
}

export function Settings() {
  const { theme, setTheme } = useTheme();
  const [a11y, setA11y] = useState<boolean | null>(null);
  const [inputMon, setInputMon] = useState<boolean | null>(null);
  const [trackStatus, setTrackStatus] = useState<TrackingStatus | null>(null);
  const [trackBusy, setTrackBusy] = useState(false);
  const [pauseBusy, setPauseBusy] = useState<number | null>(null);
  const [dailyOn, setDailyOn] = useState(true);
  const [hour, setHour] = useState(18);
  const [minute, setMinute] = useState(0);
  const [notifBusy, setNotifBusy] = useState(false);
  const [bridge, setBridge] = useState<BridgeStatus | null>(null);
  const [installOpen, setInstallOpen] = useState(false);

  const persist = useCallback(async (key: string, value: string) => {
    await invoke("set_setting", { key, value });
  }, []);

  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const ds = await invoke<string | null>("get_setting", {
          key: "daily_summary_enabled",
        });
        if (!cancelled)
          setDailyOn(ds !== "false" && ds !== "0");

        const th = await invoke<string | null>("get_setting", {
          key: "daily_summary_hour",
        });
        if (!cancelled && th != null) {
          const h = parseInt(th, 10);
          if (!Number.isNaN(h) && h >= 0 && h <= 23) setHour(h);
        }

        const tm = await invoke<string | null>("get_setting", {
          key: "daily_summary_minute",
        });
        if (!cancelled && tm != null) {
          const m = parseInt(tm, 10);
          if (!Number.isNaN(m) && [0, 15, 30, 45].includes(m)) setMinute(m);
        }
      } catch {
        /* defaults */
      }
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  useEffect(() => {
    let cancelled = false;
    const tick = async () => {
      try {
        const [a, im] = await Promise.all([
          invoke<boolean>("check_accessibility_permission"),
          invoke<boolean>("check_input_monitoring_permission"),
        ]);
        if (!cancelled) {
          setA11y(a);
          setInputMon(im);
        }
      } catch {
        if (!cancelled) {
          setA11y(null);
          setInputMon(null);
        }
      }
    };
    void tick();
    const id = window.setInterval(() => void tick(), 4000);
    return () => {
      cancelled = true;
      window.clearInterval(id);
    };
  }, []);

  useEffect(() => {
    let cancelled = false;
    const tick = () => {
      invoke<TrackingStatus>("get_tracking_status")
        .then((s) => {
          if (!cancelled) setTrackStatus(s);
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

  useEffect(() => {
    let cancelled = false;
    const tick = () => {
      invoke<BridgeStatus>("get_bridge_status")
        .then((s) => {
          if (!cancelled) setBridge(s);
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

  const toggleTracking = async () => {
    if (!trackStatus || trackBusy) return;
    setTrackBusy(true);
    try {
      if (trackStatus.running) {
        await invoke("stop_tracking");
      } else {
        await invoke("start_tracking");
      }
      const s = await invoke<TrackingStatus>("get_tracking_status");
      setTrackStatus(s);
    } finally {
      setTrackBusy(false);
    }
  };

  const pauseFor = async (minutes: number) => {
    setPauseBusy(minutes);
    try {
      await invoke("pause_tracking_for", { minutes });
      const s = await invoke<TrackingStatus>("get_tracking_status");
      setTrackStatus(s);
    } finally {
      setPauseBusy(null);
    }
  };

  const toggleDaily = async (on: boolean) => {
    setDailyOn(on);
    await persist("daily_summary_enabled", on ? "true" : "false");
  };

  const saveHour = async (h: number) => {
    setHour(h);
    await persist("daily_summary_hour", String(h));
  };

  const saveMinute = async (m: number) => {
    setMinute(m);
    await persist("daily_summary_minute", String(m));
  };

  const sendTestNotif = async () => {
    setNotifBusy(true);
    try {
      await invoke("send_test_notification");
    } finally {
      setNotifBusy(false);
    }
  };

  const br = bridgeUiStatus(bridge);

  return (
    <div className="relative min-h-full">
      <div
        aria-hidden
        className="pointer-events-none absolute inset-0 -z-10 bg-gradient-to-b from-white to-tangerine-50/30 dark:from-slate-900 dark:to-tangerine-900/20"
      />
      <div className="mx-auto max-w-2xl space-y-8 px-10 py-10">
        <header>
          <p className="text-xs font-medium uppercase tracking-[0.18em] text-tangerine-600 dark:text-tangerine-400">
            Settings
          </p>
          <h1
            className="mt-2 text-3xl tracking-tightish text-slate-900 dark:text-slate-100"
            style={{ fontWeight: 650 }}
          >
            Make Jadon's Excuses yours
          </h1>
          <p className="mt-2 text-sm text-slate-600 dark:text-slate-400">
            These changes save automatically.
          </p>
        </header>

        <SectionCard icon={Palette} eyebrow="Appearance" title="Theme">
          <div className="flex flex-wrap gap-2">
            {(
              [
                ["light", "Light"],
                ["dark", "Dark"],
                ["system", "Match system"],
              ] as const
            ).map(([k, label]) => (
              <button
                key={k}
                type="button"
                onClick={() => void setTheme(k as ThemePreference)}
                className={cn(
                  "rounded-full px-4 py-2 text-sm font-medium transition",
                  theme === k
                    ? "bg-tangerine-500 text-white"
                    : "bg-slate-100 text-slate-600 hover:bg-slate-200 dark:bg-slate-800 dark:text-slate-300 dark:hover:bg-slate-700",
                )}
              >
                {label}
              </button>
            ))}
          </div>
          <div className="mt-4 flex gap-3">
            <div className="h-14 flex-1 rounded-xl border border-slate-200 bg-white p-2 dark:border-slate-700 dark:bg-white">
              <p className="text-[10px] text-slate-600">Light</p>
              <div className="mt-1 h-6 rounded-md bg-white ring-1 ring-slate-100" />
            </div>
            <div className="h-14 flex-1 rounded-xl border border-slate-200 bg-slate-900 p-2 dark:border-slate-700">
              <p className="text-[10px] text-slate-400">Dark</p>
              <div className="mt-1 h-6 rounded-md bg-slate-900 ring-1 ring-slate-700" />
            </div>
            <div className="h-14 flex-1 overflow-hidden rounded-xl border border-slate-200 p-0 dark:border-slate-700">
              <p className="px-2 pt-2 text-[10px] text-slate-500">System</p>
              <div className="mt-1 flex h-6">
                <div className="w-1/2 bg-white" />
                <div className="w-1/2 bg-slate-900" />
              </div>
            </div>
          </div>
        </SectionCard>

        <SectionCard
          icon={ShieldCheck}
          eyebrow="Permissions"
          title="Mac privacy"
        >
          <p className="mb-4 text-sm text-slate-600 dark:text-slate-400">
            Required for tracking to work.
          </p>
          <PermissionRow
            label="Accessibility"
            ok={a11y}
            grantedLabel="Granted"
            onOpen={() => void invoke("request_accessibility_permission")}
          />
          <div className="my-4 border-t border-slate-100 dark:border-slate-800" />
          <PermissionRow
            label="Input Monitoring"
            ok={inputMon}
            grantedLabel="Granted"
            onOpen={() => void invoke("request_input_monitoring_permission")}
          />
        </SectionCard>

        <SectionCard icon={Activity} eyebrow="Tracking" title="Foreground timer">
          <div className="flex flex-wrap items-center justify-between gap-4">
            <div>
              <p className="text-sm font-medium text-slate-900 dark:text-slate-100">
                Track activity
              </p>
              <p className="text-xs text-slate-500 dark:text-slate-400">
                Pause when you need focus elsewhere — resume anytime.
              </p>
            </div>
            <Switch
              checked={trackStatus?.running ?? false}
              onChange={() => void toggleTracking()}
              disabled={trackBusy || !trackStatus}
            />
          </div>
          {trackStatus && !trackStatus.running && (
            <div className="mt-4 rounded-xl bg-slate-50 p-4 text-sm text-slate-600 dark:bg-slate-800 dark:text-slate-300">
              Tracking is paused. Your activity won't appear in the dashboard
              until you turn this back on.
            </div>
          )}
          <div className="mt-6">
            <p className="text-xs font-medium uppercase tracking-wide text-slate-500">
              Pause for…
            </p>
            <div className="mt-2 flex flex-wrap gap-2">
              {[15, 30, 60].map((m) => (
                <button
                  key={m}
                  type="button"
                  disabled={pauseBusy !== null}
                  onClick={() => void pauseFor(m)}
                  className="rounded-full bg-slate-100 px-4 py-2 text-sm font-medium text-slate-700 transition hover:bg-slate-200 disabled:opacity-50 dark:bg-slate-800 dark:text-slate-200 dark:hover:bg-slate-700"
                >
                  {pauseBusy === m ? "Pausing…" : m === 60 ? "1h" : `${m}min`}
                </button>
              ))}
            </div>
          </div>
        </SectionCard>

        <SectionCard icon={Bell} eyebrow="Notifications" title="Daily summary">
          <div className="flex flex-wrap items-center justify-between gap-4">
            <p className="text-sm text-slate-700 dark:text-slate-300">
              Daily summary at end of day
            </p>
            <Switch checked={dailyOn} onChange={(v) => void toggleDaily(v)} />
          </div>
          <div className="mt-4 flex flex-wrap items-end gap-3">
            <label className="block">
              <span className="text-xs font-medium text-slate-500">Hour</span>
              <select
                value={hour}
                onChange={(e) => void saveHour(Number(e.target.value))}
                className="mt-1 block rounded-xl border border-slate-200 bg-white px-3 py-2 text-sm dark:border-slate-700 dark:bg-slate-800 dark:text-slate-100"
              >
                {Array.from({ length: 24 }, (_, i) => (
                  <option key={i} value={i}>
                    {String(i).padStart(2, "0")}
                  </option>
                ))}
              </select>
            </label>
            <label className="block">
              <span className="text-xs font-medium text-slate-500">Minute</span>
              <select
                value={minute}
                onChange={(e) => void saveMinute(Number(e.target.value))}
                className="mt-1 block rounded-xl border border-slate-200 bg-white px-3 py-2 text-sm dark:border-slate-700 dark:bg-slate-800 dark:text-slate-100"
              >
                {[0, 15, 30, 45].map((m) => (
                  <option key={m} value={m}>
                    {String(m).padStart(2, "0")}
                  </option>
                ))}
              </select>
            </label>
            <button
              type="button"
              disabled={notifBusy}
              onClick={() => void sendTestNotif()}
              className="rounded-xl border border-slate-200 bg-white px-4 py-2 text-sm font-medium text-slate-700 shadow-sm transition hover:border-tangerine-200 hover:text-tangerine-700 disabled:opacity-50 dark:border-slate-700 dark:bg-slate-800 dark:text-slate-200 dark:hover:border-tangerine-600"
            >
              {notifBusy ? "Sending…" : "Send test notification"}
            </button>
          </div>
        </SectionCard>

        <SectionCard
          icon={Globe}
          eyebrow="Browser"
          title="Browser tab tracking"
        >
          <p className="text-sm text-slate-600 dark:text-slate-400">
            Connect Chrome to track which websites you visit, not just &quot;Google
            Chrome&quot;.
          </p>
          <div className="mt-4 flex items-center gap-2 text-sm">
            <span className={cn("h-2.5 w-2.5 shrink-0 rounded-full", br.dot)} />
            <span className="text-slate-800 dark:text-slate-200">{br.label}</span>
          </div>
          <button
            type="button"
            onClick={() => setInstallOpen((o) => !o)}
            className="mt-4 text-sm font-medium text-tangerine-600 hover:text-tangerine-700 dark:text-tangerine-400"
          >
            How to install →
          </button>
          <motion.div
            initial={false}
            animate={{ height: installOpen ? "auto" : 0, opacity: installOpen ? 1 : 0 }}
            transition={{ duration: 0.25 }}
            className="overflow-hidden"
          >
            <ol className="mt-4 list-decimal space-y-2 pl-5 text-sm text-slate-600 dark:text-slate-400">
              <li>Open chrome://extensions in Chrome</li>
              <li>Toggle Developer mode (top-right)</li>
              <li>Click Load unpacked</li>
              <li>
                Select the <code className="rounded bg-slate-100 px-1 dark:bg-slate-800">browser-extension</code>{" "}
                folder in this project
              </li>
              <li className="break-all font-mono text-xs text-slate-500">
                /Users/noahmanning/Code/jadons-excuses/browser-extension
              </li>
            </ol>
          </motion.div>
        </SectionCard>

        <SectionCard icon={Info} eyebrow="About" title="Jadon's Excuses">
          <p className="text-sm font-medium text-slate-900 dark:text-slate-100">
            v0.1.0
          </p>
          <p className="mt-2 text-sm text-slate-600 dark:text-slate-400">
            100% local — your data never leaves this Mac
          </p>
          <Link
            to="/settings/advanced"
            className="mt-6 inline-block text-xs font-medium text-slate-500 underline-offset-4 hover:text-tangerine-600 hover:underline dark:text-slate-500 dark:hover:text-tangerine-400"
          >
            View advanced/developer tools →
          </Link>
        </SectionCard>
      </div>
    </div>
  );
}

function PermissionRow({
  label,
  ok,
  grantedLabel,
  onOpen,
}: {
  label: string;
  ok: boolean | null;
  grantedLabel: string;
  onOpen: () => void;
}) {
  return (
    <div className="flex flex-wrap items-center justify-between gap-3">
      <span className="text-sm font-medium text-slate-900 dark:text-slate-100">
        {label}
      </span>
      <div className="flex items-center gap-2">
        {ok === null ? (
          <span className="text-xs text-slate-400">Checking…</span>
        ) : ok ? (
          <span className="inline-flex items-center gap-1.5 text-sm text-emerald-600 dark:text-emerald-400">
            <CheckCircle className="h-4 w-4" />
            {grantedLabel}
          </span>
        ) : (
          <>
            <span className="inline-flex items-center gap-1.5 text-sm text-slate-400">
              <XCircle className="h-4 w-4" />
              Needed
            </span>
            <button
              type="button"
              onClick={onOpen}
              className="rounded-lg border border-slate-200 bg-white px-3 py-1.5 text-xs font-medium text-slate-700 shadow-sm hover:border-tangerine-200 dark:border-slate-700 dark:bg-slate-800 dark:text-slate-200"
            >
              Open System Settings
            </button>
          </>
        )}
      </div>
    </div>
  );
}
