import { useEffect, useMemo, useState } from "react";
import { motion } from "framer-motion";
import CountUp from "react-countup";
import { Link } from "react-router-dom";
import { invoke } from "@tauri-apps/api/core";
import {
  Activity as ActivityIcon,
  Clock,
  Pause,
  RefreshCw,
  Sparkles,
} from "lucide-react";
import {
  Bar,
  BarChart,
  CartesianGrid,
  Cell,
  ResponsiveContainer,
  Tooltip,
  XAxis,
  YAxis,
} from "recharts";
import {
  useDashboardData,
  type DashboardData,
  type HourPoint,
  type TopActivity,
} from "../hooks/useDashboardData";
import {
  formatDuration,
  formatDurationLive,
  formatDurationShort,
  formatPercent,
} from "../lib/formatters";
import { cn } from "../lib/utils";
import { chartPalette, useTheme } from "../contexts/ThemeProvider";

// ---------------------------------------------------------------------------
// Color tokens — kept in one place so a Tailwind theme change later only
// has to touch this file. The hex values mirror the named colors in
// `tailwind.config.js` (tangerine, slate, success, danger).
// ---------------------------------------------------------------------------

const COLORS = {
  tangerine: "#F28500",
  tangerineLight: "#FCE5C0",
  slate900: "#1A1A1A",
  slate600: "#4B4F53",
  slate400: "#797D81",
  slate300: "#cbd5e1",
  slate100: "#F5F5F7",
  success: "#10B981",
  danger: "#EF4444",
  light: "#3B82F6", // engagement bucket: 1-25
  passive: "#F59E0B", // engagement bucket: 26-50
} as const;

function useLiveEngagementState(): string {
  const [state, setState] = useState("idle");
  useEffect(() => {
    let cancelled = false;
    const poll = async () => {
      try {
        const e = await invoke<{ current_state?: string }>(
          "get_current_engagement",
        );
        if (!cancelled) setState(e.current_state ?? "idle");
      } catch {
        if (!cancelled) setState("idle");
      }
    };
    void poll();
    const id = window.setInterval(() => void poll(), 1000);
    return () => {
      cancelled = true;
      window.clearInterval(id);
    };
  }, []);
  return state;
}

function useTickingCounter(
  baseSeconds: number,
  dataFetchedAt: number,
  isLive: boolean,
) {
  const [tick, setTick] = useState(0);
  useEffect(() => {
    if (!isLive) return;
    const id = window.setInterval(() => setTick((t) => t + 1), 1000);
    return () => window.clearInterval(id);
  }, [isLive, dataFetchedAt]);
  return useMemo(() => {
    if (!isLive) return baseSeconds;
    const delta = Math.floor((Date.now() - dataFetchedAt) / 1000);
    return baseSeconds + Math.max(0, delta);
  }, [baseSeconds, dataFetchedAt, isLive, tick]);
}

// ---------------------------------------------------------------------------

export function Dashboard() {
  const data = useDashboardData();
  const { overview, topActivity, isLoading, error } = data;

  // First-ever launch: no events tracked yet at all → friendly welcome.
  const isEmpty =
    overview !== null &&
    overview.tracked_seconds === 0 &&
    topActivity.length === 0;

  return (
    <div className="relative min-h-full">
      <div
        aria-hidden
        className="pointer-events-none absolute inset-0 -z-10 bg-gradient-to-b from-white to-tangerine-50/30 dark:from-slate-900 dark:to-tangerine-900/20"
      />

      <div className="mx-auto flex max-w-6xl flex-col gap-6 px-10 py-12">
        {error && (
          <div className="rounded-xl border border-red-200 bg-red-50 p-4 text-sm text-red-700 dark:border-red-900/50 dark:bg-red-950/40 dark:text-red-300">
            <p className="font-medium">Couldn't load dashboard data.</p>
            <p className="mt-1 font-mono text-xs">{error}</p>
          </div>
        )}

        {isEmpty ? (
          <EmptyState />
        ) : (
          <>
            <HeroCard data={data} delay={0} />
            <div className="grid grid-cols-1 gap-6 md:grid-cols-2">
              <MostUsedAppCard
                top={topActivity[0] ?? null}
                loading={isLoading && topActivity.length === 0}
                delay={0.08}
              />
              <SwitchesCard
                overview={overview}
                loading={isLoading && overview === null}
                delay={0.16}
              />
            </div>
            <HourlyChartCard data={data} delay={0.24} />
            <TopAppsCard data={data} delay={0.32} />
          </>
        )}
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Section 1 — Hero card with focus-score ring + tracked/active/idle stats
// ---------------------------------------------------------------------------

function HeroCard({ data, delay }: { data: DashboardData; delay: number }) {
  const { overview, isLoading, isFetching, dataFetchedAt, refresh } = data;
  const showSkeleton = isLoading && overview === null;
  const engState = useLiveEngagementState();
  const isIdle = engState === "idle";

  const trackedLive = useTickingCounter(
    overview?.tracked_seconds ?? 0,
    dataFetchedAt,
    true,
  );
  const activeLive = useTickingCounter(
    overview?.active_seconds ?? 0,
    dataFetchedAt,
    !isIdle,
  );
  const idleLive = useTickingCounter(
    overview?.idle_seconds ?? 0,
    dataFetchedAt,
    isIdle,
  );

  return (
    <motion.section
      initial={{ opacity: 0, y: 8 }}
      animate={{ opacity: 1, y: 0 }}
      transition={{ duration: 0.4, ease: "easeOut", delay }}
      className="relative overflow-hidden rounded-2xl border border-tangerine-100 bg-white/80 p-8 shadow-soft backdrop-blur dark:border-tangerine-900/40 dark:bg-slate-900/80"
    >
      <div
        aria-hidden
        className="pointer-events-none absolute -right-12 -top-12 h-56 w-56 rounded-full"
        style={{
          background:
            "radial-gradient(closest-side, rgba(242,133,0,0.12), transparent)",
        }}
      />
      <div className="mb-5 flex items-center justify-between gap-3">
        <div className="inline-flex items-center gap-2 rounded-full border border-tangerine-100 bg-tangerine-50 px-3 py-1 text-xs font-medium text-tangerine-700 dark:border-tangerine-900/50 dark:bg-tangerine-950/40 dark:text-tangerine-300">
          <Sparkles className="h-3.5 w-3.5" />
          Today
        </div>
        <button
          type="button"
          title="Refresh now"
          onClick={() => refresh()}
          className={cn(
            "rounded-lg p-2 text-slate-500 transition-colors hover:bg-slate-100 hover:text-slate-700 dark:text-slate-400 dark:hover:bg-slate-800 dark:hover:text-slate-200",
            isFetching && "animate-spin",
          )}
        >
          <RefreshCw className="h-4 w-4" aria-hidden />
          <span className="sr-only">Refresh now</span>
        </button>
      </div>

      <div className="grid grid-cols-1 items-center gap-8 md:grid-cols-[auto_1fr]">
        <div className="flex flex-col items-center md:items-start">
          {showSkeleton ? (
            <div className="h-[180px] w-[180px] animate-pulse rounded-full bg-slate-100" />
          ) : (
            <FocusScoreRing overview={overview} />
          )}
          <p className="mt-3 max-w-xs text-center text-sm text-slate-600 md:text-left dark:text-slate-400">
            Receipts for your day.
          </p>
        </div>

        <div className="grid grid-cols-1 gap-3 sm:grid-cols-3">
          <HeroStat
            icon={<Clock className="h-4 w-4" />}
            label="Tracked Today"
            displaySeconds={trackedLive}
            tone="slate"
            loading={showSkeleton}
          />
          <HeroStat
            icon={<ActivityIcon className="h-4 w-4" />}
            label="Active"
            displaySeconds={activeLive}
            tone="success"
            loading={showSkeleton}
          />
          <HeroStat
            icon={<Pause className="h-4 w-4" />}
            label="Idle"
            displaySeconds={idleLive}
            tone="muted"
            loading={showSkeleton}
          />
        </div>
      </div>
    </motion.section>
  );
}

function focusRingLabel(
  insufficientData: boolean,
  clamped: number,
): { numberText: string; descriptor: string } {
  if (insufficientData) {
    return { numberText: "—", descriptor: "Not enough data yet" };
  }
  if (clamped >= 60) {
    return { numberText: String(clamped), descriptor: "Highly focused" };
  }
  if (clamped >= 30) {
    return { numberText: String(clamped), descriptor: "Steady" };
  }
  return { numberText: String(clamped), descriptor: "Distracted" };
}

function FocusScoreRing({
  overview,
}: {
  overview: DashboardData["overview"];
}) {
  const { resolvedTheme } = useTheme();
  const trackStroke =
    resolvedTheme === "dark" ? "#1e293b" : COLORS.slate100;
  const radius = 80;
  const stroke = 12;
  const size = 180;
  const center = size / 2;
  const circumference = 2 * Math.PI * radius;

  const tracked = overview?.tracked_seconds ?? 0;
  const samples = overview?.engagement_sample_count ?? 0;
  const insufficientData = tracked === 0 || samples === 0;
  const rawScore = overview?.focus_score ?? 0;
  const clamped = Math.max(0, Math.min(100, insufficientData ? 0 : rawScore));
  const { numberText, descriptor } = focusRingLabel(insufficientData, clamped);

  const color =
    insufficientData
      ? COLORS.slate400
      : clamped >= 60
        ? COLORS.tangerine
        : clamped >= 30
          ? COLORS.slate400
          : COLORS.danger;

  const offset = circumference * (1 - clamped / 100);

  return (
    <div className="relative h-[180px] w-[180px] shrink-0">
      <svg
        className="absolute inset-0 h-full w-full"
        width={size}
        height={size}
        viewBox={`0 0 ${size} ${size}`}
        aria-hidden
      >
        <circle
          cx={center}
          cy={center}
          r={radius}
          stroke={trackStroke}
          strokeWidth={stroke}
          fill="none"
        />
        {!insufficientData && (
          <motion.circle
            cx={center}
            cy={center}
            r={radius}
            stroke={color}
            strokeWidth={stroke}
            fill="none"
            strokeDasharray={circumference}
            initial={{ strokeDashoffset: circumference }}
            animate={{ strokeDashoffset: offset }}
            transition={{ duration: 1.1, ease: "easeOut" }}
            strokeLinecap="round"
            transform={`rotate(-90 ${center} ${center})`}
          />
        )}
      </svg>

      <div className="pointer-events-none absolute inset-0 flex flex-col items-center justify-center px-5 text-center">
        <motion.span
          key={`${numberText}-${descriptor}`}
          className={cn(
            "text-4xl tabular-nums tracking-tight text-slate-900 dark:text-slate-100",
            insufficientData && "text-slate-400 dark:text-slate-500",
          )}
          style={{ fontWeight: 700 }}
          initial={{ opacity: 0 }}
          animate={{ opacity: 1 }}
          transition={{ duration: 0.35 }}
        >
          {numberText}
        </motion.span>
        <span
          className="mt-1 text-[11px] font-medium uppercase tracking-[0.18em] text-slate-500 dark:text-slate-400"
          style={{ fontWeight: 500 }}
        >
          Focus Score
        </span>
        <span className="mt-1 max-w-[11rem] text-xs leading-snug text-slate-500 dark:text-slate-400">
          {descriptor}
        </span>
      </div>
    </div>
  );
}

function HeroStat({
  icon,
  label,
  displaySeconds,
  tone,
  loading,
}: {
  icon: React.ReactNode;
  label: string;
  displaySeconds: number;
  tone: "slate" | "success" | "muted";
  loading: boolean;
}) {
  const valueClass =
    tone === "success"
      ? "text-emerald-600 dark:text-emerald-400"
      : tone === "muted"
        ? "text-slate-400 dark:text-slate-500"
        : "text-slate-900 dark:text-slate-100";
  const iconClass =
    tone === "success"
      ? "text-emerald-500"
      : tone === "muted"
        ? "text-slate-400"
        : "text-tangerine-600";

  return (
    <div className="rounded-xl border border-slate-100 bg-white/90 p-4 dark:border-slate-800 dark:bg-slate-800/50">
      <div className="flex items-center gap-2 text-xs font-medium uppercase tracking-[0.14em] text-slate-500 dark:text-slate-400">
        <span className={iconClass}>{icon}</span>
        {label}
      </div>
      {loading ? (
        <div className="mt-2 h-9 w-20 animate-pulse rounded bg-slate-100" />
      ) : (
        <p
          className={cn(
            "mt-2 text-3xl tabular-nums tracking-tightish",
            valueClass,
          )}
          style={{ fontWeight: 650 }}
        >
          {formatDurationLive(displaySeconds)}
        </p>
      )}
    </div>
  );
}

// ---------------------------------------------------------------------------
// Section 2a — Most Used App
// ---------------------------------------------------------------------------

function MostUsedAppCard({
  top,
  loading,
  delay,
}: {
  top: TopActivity | null;
  loading: boolean;
  delay: number;
}) {
  return (
    <motion.section
      initial={{ opacity: 0, y: 8 }}
      animate={{ opacity: 1, y: 0 }}
      transition={{ duration: 0.3, ease: "easeOut", delay }}
      className="rounded-2xl border border-slate-100 bg-white p-6 shadow-soft dark:border-slate-800 dark:bg-slate-900"
    >
      <p className="text-xs font-medium uppercase tracking-[0.18em] text-slate-500 dark:text-slate-400">
        Most Used App
      </p>
      {loading ? (
        <div className="mt-4 space-y-3">
          <div className="h-7 w-2/3 animate-pulse rounded bg-slate-100" />
          <div className="h-9 w-1/2 animate-pulse rounded bg-slate-100" />
          <div className="h-3 w-full animate-pulse rounded bg-slate-100" />
        </div>
      ) : top ? (
        <>
          <h3
            className="mt-3 truncate text-2xl tracking-tightish text-slate-900 dark:text-slate-100"
            style={{ fontWeight: 650 }}
            title={top.name}
          >
            {top.kind === "website" ? "🌐 " : null}
            {top.name}
          </h3>
          <p
            className="mt-2 text-3xl tabular-nums text-slate-900 dark:text-slate-100"
            style={{ fontWeight: 700 }}
          >
            {formatDuration(top.total_seconds)}
          </p>
          <div className="mt-3 flex items-center gap-2 text-xs text-slate-600 dark:text-slate-400">
            <CategoryBadge
              name={top.category_name ?? "Uncategorized"}
              color={top.category_color ?? null}
            />
          </div>
          <div className="mt-4">
            <div className="mb-1 flex items-center justify-between text-xs text-slate-500 dark:text-slate-400">
              <span>Engagement</span>
              <span className="tabular-nums text-slate-700 dark:text-slate-300">
                {formatPercent(top.avg_engagement)}
              </span>
            </div>
            <div className="h-2 w-full overflow-hidden rounded-full bg-slate-100 dark:bg-slate-800">
              <motion.div
                className="h-full rounded-full"
                style={{ background: COLORS.tangerine }}
                initial={{ width: 0 }}
                animate={{
                  width: `${Math.max(0, Math.min(100, top.avg_engagement))}%`,
                }}
                transition={{ duration: 0.8, ease: "easeOut" }}
              />
            </div>
          </div>
        </>
      ) : (
        <p className="mt-3 text-sm text-slate-400">
          Nothing tracked yet today.
        </p>
      )}
    </motion.section>
  );
}

// ---------------------------------------------------------------------------
// Section 2b — Context Switches
// ---------------------------------------------------------------------------

function SwitchesCard({
  overview,
  loading,
  delay,
}: {
  overview: DashboardData["overview"];
  loading: boolean;
  delay: number;
}) {
  // Average seconds per focus block. Falls back to the "—" copy when
  // we can't compute it (zero switches OR zero tracked time).
  const avgSecondsPerBlock = useMemo(() => {
    if (!overview) return null;
    if (overview.switch_count === 0) return null;
    if (overview.tracked_seconds === 0) return null;
    return Math.round(overview.tracked_seconds / overview.switch_count);
  }, [overview]);

  const fragLabel = useMemo(() => {
    const f = overview?.fragmentation_score ?? 0;
    if (f < 30) return { emoji: "🟢", text: "Focused", tone: "good" } as const;
    if (f <= 60)
      return { emoji: "🟡", text: "Some switching", tone: "warn" } as const;
    return { emoji: "🔴", text: "Highly fragmented", tone: "bad" } as const;
  }, [overview]);

  return (
    <motion.section
      initial={{ opacity: 0, y: 8 }}
      animate={{ opacity: 1, y: 0 }}
      transition={{ duration: 0.3, ease: "easeOut", delay }}
      className="rounded-2xl border border-slate-100 bg-white p-6 shadow-soft dark:border-slate-800 dark:bg-slate-900"
    >
      <p className="text-xs font-medium uppercase tracking-[0.18em] text-slate-500 dark:text-slate-400">
        Context Switches
      </p>
      {loading ? (
        <div className="mt-4 space-y-3">
          <div className="h-12 w-32 animate-pulse rounded bg-slate-100" />
          <div className="h-4 w-3/4 animate-pulse rounded bg-slate-100" />
          <div className="h-4 w-1/3 animate-pulse rounded bg-slate-100" />
        </div>
      ) : (
        <>
          <p
            className="mt-3 text-5xl tabular-nums text-slate-900 dark:text-slate-100"
            style={{ fontWeight: 700, letterSpacing: "-0.02em" }}
          >
            <CountUp end={overview?.switch_count ?? 0} duration={1.0} preserveValue />
          </p>
          <p className="mt-3 text-sm text-slate-600 dark:text-slate-400">
            {avgSecondsPerBlock === null
              ? "No switches yet today."
              : `You jumped between apps every ${formatDuration(
                  avgSecondsPerBlock,
                )} on average.`}
          </p>
          <div
            className={cn(
              "mt-4 inline-flex items-center gap-2 rounded-full px-3 py-1 text-xs font-medium",
              fragLabel.tone === "good" &&
                "bg-emerald-50 text-emerald-700 dark:bg-emerald-950/40 dark:text-emerald-300",
              fragLabel.tone === "warn" &&
                "bg-amber-50 text-amber-700 dark:bg-amber-950/40 dark:text-amber-300",
              fragLabel.tone === "bad" &&
                "bg-red-50 text-red-700 dark:bg-red-950/40 dark:text-red-300",
            )}
          >
            <span aria-hidden>{fragLabel.emoji}</span>
            {fragLabel.text}
            <span className="text-slate-500 dark:text-slate-400">
              · {overview?.fragmentation_score ?? 0}/100
            </span>
          </div>
        </>
      )}
    </motion.section>
  );
}

// ---------------------------------------------------------------------------
// Section 3 — Hourly engagement chart
// ---------------------------------------------------------------------------

/** Maps an hour-bucket avg score to its bar color. Mirrors `score_to_state`. */
function colorForHour(point: HourPoint): string {
  if (point.active_minutes === 0 || point.avg_engagement === 0) {
    return COLORS.slate300;
  }
  const s = point.avg_engagement;
  if (s <= 25) return COLORS.light;
  if (s <= 50) return COLORS.passive;
  if (s <= 80) return COLORS.success;
  return COLORS.tangerine;
}

function HourlyChartCard({
  data,
  delay,
}: {
  data: DashboardData;
  delay: number;
}) {
  const { hourly, isLoading } = data;
  const { resolvedTheme } = useTheme();
  const pal = chartPalette(resolvedTheme);
  const showSkeleton = isLoading && hourly.length === 0;

  // Tick labels: only render the four "anchor" hours so the axis stays
  // legible at every viewport width.
  const xTickFormatter = (h: number) => {
    if (h === 0) return "12am";
    if (h === 6) return "6am";
    if (h === 12) return "12pm";
    if (h === 18) return "6pm";
    return "";
  };

  return (
    <motion.section
      initial={{ opacity: 0, y: 8 }}
      animate={{ opacity: 1, y: 0 }}
      transition={{ duration: 0.3, ease: "easeOut", delay }}
      className="rounded-2xl border border-slate-100 bg-white p-6 shadow-soft dark:border-slate-800 dark:bg-slate-900"
    >
      <div className="mb-1 flex items-baseline justify-between">
        <h3
          className="text-lg tracking-tightish text-slate-900 dark:text-slate-100"
          style={{ fontWeight: 650 }}
        >
          Your Day, Hour by Hour
        </h3>
      </div>
      <p className="mb-4 text-sm text-slate-600 dark:text-slate-400">
        Engagement intensity from when you started.
      </p>
      <div className="h-56 w-full">
        {showSkeleton ? (
          <div className="h-full w-full animate-pulse rounded bg-slate-50 dark:bg-slate-800/60" />
        ) : (
          <ResponsiveContainer width="100%" height="100%">
            <BarChart
              data={hourly}
              margin={{ top: 8, right: 8, left: -16, bottom: 0 }}
            >
              <CartesianGrid
                strokeDasharray="3 3"
                stroke={pal.grid}
                vertical={false}
              />
              <XAxis
                dataKey="hour"
                stroke={pal.tick}
                tick={{ fontSize: 11, fill: pal.tick }}
                tickLine={false}
                axisLine={false}
                tickFormatter={xTickFormatter}
                interval={0}
              />
              <YAxis
                domain={[0, 100]}
                stroke={pal.tick}
                tick={{ fontSize: 11, fill: pal.tick }}
                tickLine={false}
                axisLine={false}
                width={32}
              />
              <Tooltip
                cursor={{ fill: "rgba(242, 133, 0, 0.06)" }}
                content={<HourTooltip />}
              />
              <Bar
                dataKey="avg_engagement"
                radius={[4, 4, 0, 0]}
                animationDuration={800}
              >
                {hourly.map((point, i) => (
                  <Cell key={i} fill={colorForHour(point)} />
                ))}
              </Bar>
            </BarChart>
          </ResponsiveContainer>
        )}
      </div>
      <ChartLegend />
    </motion.section>
  );
}

function HourTooltip({
  active,
  payload,
}: {
  active?: boolean;
  payload?: Array<{ payload: HourPoint }>;
}) {
  if (!active || !payload || payload.length === 0) return null;
  const p = payload[0].payload;
  const hourLabel = (h: number) => {
    if (h === 0) return "12 – 1 AM";
    if (h < 12) return `${h} – ${h + 1} AM`;
    if (h === 12) return "12 – 1 PM";
    return `${h - 12} – ${h - 11} PM`;
  };
  return (
    <div
      className={cn(
        "rounded-lg border px-3 py-2 text-xs shadow-soft",
        "border-slate-200 bg-white dark:border-slate-700 dark:bg-slate-800",
      )}
    >
      <p className="font-medium text-slate-900 dark:text-slate-100">{hourLabel(p.hour)}</p>
      <p className="text-slate-600 dark:text-slate-400">
        Avg engagement:{" "}
        <span className="font-medium text-slate-900 dark:text-slate-100">
          {p.avg_engagement}/100
        </span>
      </p>
      <p className="text-slate-600 dark:text-slate-400">
        Active:{" "}
        <span className="font-medium text-slate-900 dark:text-slate-100">
          {p.active_minutes} min
        </span>
      </p>
    </div>
  );
}

function ChartLegend() {
  const items: Array<{ color: string; label: string }> = [
    { color: COLORS.slate300, label: "Idle" },
    { color: COLORS.light, label: "Light" },
    { color: COLORS.passive, label: "Passive" },
    { color: COLORS.success, label: "Active" },
    { color: COLORS.tangerine, label: "Intense" },
  ];
  return (
    <div className="mt-4 flex flex-wrap gap-x-4 gap-y-2 text-xs text-slate-500 dark:text-slate-400">
      {items.map((it) => (
        <div key={it.label} className="flex items-center gap-1.5">
          <span
            className="inline-block h-2 w-2 rounded-full"
            style={{ background: it.color }}
          />
          {it.label}
        </div>
      ))}
    </div>
  );
}

// ---------------------------------------------------------------------------
// Section 4 — Top Apps with active/total split bar
// ---------------------------------------------------------------------------

function TopAppsCard({ data, delay }: { data: DashboardData; delay: number }) {
  const { topActivity, isLoading } = data;
  const showSkeleton = isLoading && topActivity.length === 0;

  const maxTotal = useMemo(() => {
    if (topActivity.length === 0) return 1;
    return Math.max(1, ...topActivity.map((a) => a.total_seconds));
  }, [topActivity]);

  return (
    <motion.section
      initial={{ opacity: 0, y: 8 }}
      animate={{ opacity: 1, y: 0 }}
      transition={{ duration: 0.3, ease: "easeOut", delay }}
      className="rounded-2xl border border-slate-100 bg-white p-6 shadow-soft dark:border-slate-800 dark:bg-slate-900"
    >
      <h3
        className="text-lg tracking-tightish text-slate-900 dark:text-slate-100"
        style={{ fontWeight: 650 }}
      >
        Where Your Day Went
      </h3>
      <p className="mb-5 text-sm text-slate-600 dark:text-slate-400">
        Apps and websites by time spent today.
      </p>

      {showSkeleton ? (
        <div className="space-y-4">
          {Array.from({ length: 4 }).map((_, i) => (
            <div key={i} className="space-y-2">
              <div className="flex items-center justify-between">
                <div className="h-4 w-1/3 animate-pulse rounded bg-slate-100 dark:bg-slate-800" />
                <div className="h-4 w-12 animate-pulse rounded bg-slate-100 dark:bg-slate-800" />
              </div>
              <div className="h-3 w-full animate-pulse rounded bg-slate-100 dark:bg-slate-800" />
            </div>
          ))}
        </div>
      ) : topActivity.length === 0 ? (
        <p className="text-sm text-slate-400">
          Nothing to show yet. Start working — your day&apos;s receipts will
          appear here.
        </p>
      ) : (
        <ul className="space-y-4">
          {topActivity.map((row, i) => (
            <TopActivityRow
              key={`${row.kind}-${row.name}-${i}`}
              row={row}
              maxTotal={maxTotal}
              index={i}
            />
          ))}
        </ul>
      )}
    </motion.section>
  );
}

function dashboardActivityHref(row: TopActivity): string {
  if (row.kind === "app" && row.name === "Browser (untracked)") {
    return "/activity?view=domain";
  }
  if (row.kind === "app") {
    return `/activity?view=app&filter=${encodeURIComponent(row.name)}`;
  }
  return `/activity?view=domain&filter=${encodeURIComponent(row.name)}`;
}

function TopActivityRow({
  row,
  maxTotal,
  index,
}: {
  row: TopActivity;
  maxTotal: number;
  index: number;
}) {
  const { resolvedTheme } = useTheme();
  const trackBg =
    resolvedTheme === "dark" ? "#1e293b" : COLORS.slate100;
  const totalPct = Math.min(100, (row.total_seconds / maxTotal) * 100);
  const activePct = Math.min(100, (row.active_seconds / maxTotal) * 100);
  const isWeb = row.kind === "website";
  const hint = row.icon_hint ?? row.name;
  const initial = hint.charAt(0).toUpperCase();
  const faviconUrl =
    isWeb && row.icon_hint
      ? `https://www.google.com/s2/favicons?domain=${encodeURIComponent(row.icon_hint)}&sz=32`
      : null;

  return (
    <motion.li
      initial={{ opacity: 0, x: -8 }}
      animate={{ opacity: 1, x: 0 }}
      transition={{ duration: 0.25, delay: index * 0.04, ease: "easeOut" }}
      className="space-y-2"
    >
      <Link
        to={dashboardActivityHref(row)}
        className="-mx-1 block cursor-pointer space-y-2 rounded-lg px-1 py-0.5 outline-none ring-offset-2 ring-offset-white transition-colors hover:bg-slate-50 focus-visible:ring-2 focus-visible:ring-tangerine-400 dark:ring-offset-slate-900 dark:hover:bg-slate-800/50"
      >
      <div className="flex items-center justify-between gap-3">
        <div className="flex min-w-0 flex-1 items-center gap-2">
          <span
            className={cn(
              "flex h-8 w-8 shrink-0 items-center justify-center overflow-hidden rounded-full border border-slate-200 bg-white text-xs font-semibold text-tangerine-600 dark:border-slate-700 dark:bg-slate-800",
              isWeb && "border-tangerine-100 bg-tangerine-50/50 dark:border-tangerine-900/50 dark:bg-tangerine-950/30",
            )}
            title={row.name}
          >
            {faviconUrl ? (
              <img
                src={faviconUrl}
                alt=""
                className="h-5 w-5"
                loading="lazy"
                referrerPolicy="no-referrer"
              />
            ) : (
              initial
            )}
          </span>
          <span
            className="truncate text-sm font-medium text-slate-900 dark:text-slate-100"
            title={row.name}
          >
            {isWeb ? "🌐 " : null}
            {row.name}
          </span>
          <CategoryBadge
            name={row.category_name ?? "Uncategorized"}
            color={row.category_color ?? null}
          />
        </div>
        <span className="shrink-0 tabular-nums text-sm text-slate-700 dark:text-slate-300">
          {formatDurationShort(row.total_seconds)}
        </span>
      </div>

      <div
        className="relative h-2.5 w-full overflow-hidden rounded-full"
        style={{ background: trackBg }}
        title={`Active ${formatDurationShort(row.active_seconds)} of ${formatDurationShort(
          row.total_seconds,
        )} total`}
      >
        <motion.div
          className="absolute inset-y-0 left-0 rounded-full"
          style={{ background: COLORS.tangerineLight }}
          initial={{ width: 0 }}
          animate={{ width: `${totalPct}%` }}
          transition={{ duration: 0.7, delay: index * 0.04, ease: "easeOut" }}
        />
        <motion.div
          className="absolute inset-y-0 left-0 rounded-full"
          style={{ background: COLORS.tangerine }}
          initial={{ width: 0 }}
          animate={{ width: `${activePct}%` }}
          transition={{
            duration: 0.7,
            delay: index * 0.04 + 0.1,
            ease: "easeOut",
          }}
        />
      </div>
      </Link>
    </motion.li>
  );
}

// ---------------------------------------------------------------------------
// Shared bits
// ---------------------------------------------------------------------------

function CategoryBadge({
  name,
  color,
}: {
  name: string;
  color: string | null;
}) {
  const swatch = color ?? COLORS.slate400;
  return (
    <span className="inline-flex items-center gap-1.5 rounded-full bg-slate-50 px-2 py-0.5 text-[11px] font-medium text-slate-600 dark:bg-slate-800 dark:text-slate-300">
      <span
        aria-hidden
        className="inline-block h-1.5 w-1.5 rounded-full"
        style={{ background: swatch }}
      />
      {name}
    </span>
  );
}

function EmptyState() {
  return (
    <motion.section
      initial={{ opacity: 0, y: 8 }}
      animate={{ opacity: 1, y: 0 }}
      transition={{ duration: 0.4, ease: "easeOut" }}
      className="rounded-2xl border border-tangerine-100 bg-white/80 p-12 shadow-soft backdrop-blur dark:border-tangerine-900/40 dark:bg-slate-900/80"
    >
      <div className="mb-6 inline-flex items-center gap-2 rounded-full border border-tangerine-100 bg-tangerine-50 px-3 py-1 text-xs font-medium text-tangerine-700 dark:border-tangerine-900/50 dark:bg-tangerine-950/40 dark:text-tangerine-300">
        <Sparkles className="h-3.5 w-3.5" />
        New here
      </div>
      <h1
        className="text-5xl tracking-tightish text-tangerine-500"
        style={{ fontWeight: 700 }}
      >
        Welcome to Jadon's Excuses.
      </h1>
      <div className="mt-6 flex flex-wrap gap-2">
        <span className="rounded-full bg-emerald-50 px-3 py-1 text-xs font-medium text-emerald-800 ring-1 ring-emerald-100 dark:bg-emerald-950/40 dark:text-emerald-300 dark:ring-emerald-900/50">
          Tracking starts automatically
        </span>
        <span className="rounded-full bg-slate-50 px-3 py-1 text-xs font-medium text-slate-700 ring-1 ring-slate-200 dark:bg-slate-800 dark:text-slate-200 dark:ring-slate-700">
          Grant Accessibility + Input Monitoring (see Settings)
        </span>
        <span className="rounded-full bg-amber-50 px-3 py-1 text-xs font-medium text-amber-900 ring-1 ring-amber-100 dark:bg-amber-950/40 dark:text-amber-200 dark:ring-amber-900/50">
          Browser extension optional — see /browser-extension/README.md
        </span>
      </div>
      <p className="mt-4 max-w-2xl text-lg leading-relaxed text-slate-600 dark:text-slate-400">
        Start working — the receipts will appear here within about ten minutes
        once we have engagement samples and window events to roll up.
      </p>
      <p className="mt-6 text-sm text-slate-500 dark:text-slate-400">
        If this is your first launch, double-check that Accessibility and
        Input Monitoring are granted in System Settings → Privacy &amp;
        Security. The Settings page has a debug panel for both.
      </p>
    </motion.section>
  );
}
