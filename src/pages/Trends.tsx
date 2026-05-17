import { useEffect, useMemo, useState, type ReactNode } from "react";
import { invoke } from "@tauri-apps/api/core";
import { motion } from "framer-motion";
import {
  Area,
  AreaChart,
  Bar,
  BarChart,
  CartesianGrid,
  Cell,
  Line,
  LineChart,
  ResponsiveContainer,
  Tooltip,
  XAxis,
  YAxis,
} from "recharts";
import { formatDuration } from "../lib/formatters";
import { chartPalette, useTheme } from "../contexts/ThemeProvider";

interface DailySummaryRecord {
  date: string;
  total_active_seconds: number;
  total_idle_seconds: number;
  focus_score: number;
  longest_streak_seconds: number;
  total_switches: number;
  top_app: string | null;
  updated_at: number;
}

interface TrendsOverview {
  daily_summaries: DailySummaryRecord[];
  focus_score_trend: { date: string; score: number }[];
  avg_streak_trend: { date: string; seconds: number }[];
  category_mix: {
    date: string;
    focus_seconds: number;
    work_seconds: number;
    distracting_seconds: number;
    personal_seconds: number;
  }[];
  personal_records: {
    longest_streak_ever: number;
    best_day: { date: string; focus_score: number };
  };
}

interface HeatmapCell {
  day_of_week: number;
  hour: number;
  avg_engagement: number;
  sample_count: number;
}

const DOW = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];

function switchColor(n: number): string {
  if (n < 40) return "#10B981";
  if (n < 120) return "#F59E0B";
  return "#EF4444";
}

export function Trends() {
  const [range, setRange] = useState<7 | 30 | 90>(30);
  const [overview, setOverview] = useState<TrendsOverview | null>(null);
  const [heatmap, setHeatmap] = useState<HeatmapCell[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const { resolvedTheme } = useTheme();
  const pal = chartPalette(resolvedTheme);

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    Promise.all([
      invoke<TrendsOverview>("get_trends_overview", { days: range }),
      invoke<HeatmapCell[]>("get_weekly_heatmap"),
    ])
      .then(([ov, hm]) => {
        if (!cancelled) {
          setOverview(ov);
          setHeatmap(hm);
          setError(null);
        }
      })
      .catch((e) => {
        if (!cancelled) setError(String(e));
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [range]);

  const switchesData = useMemo(() => {
    if (!overview) return [];
    return [...overview.daily_summaries]
      .reverse()
      .map((d) => ({
        date: d.date,
        switches: d.total_switches,
        fill: switchColor(d.total_switches),
      }));
  }, [overview]);

  const heatmapMap = useMemo(() => {
    const m = new Map<string, HeatmapCell>();
    for (const c of heatmap) {
      m.set(`${c.day_of_week}-${c.hour}`, c);
    }
    return m;
  }, [heatmap]);

  return (
    <div className="min-h-full bg-gradient-to-b from-white to-tangerine-50/30 px-10 py-10 dark:from-slate-900 dark:to-tangerine-900/20">
      <div className="mx-auto max-w-6xl space-y-6">
        <header className="flex flex-col gap-3 md:flex-row md:items-end md:justify-between">
          <div>
            <p className="text-xs font-medium uppercase tracking-[0.18em] text-tangerine-600 dark:text-tangerine-400">
              Trends
            </p>
            <h1
              className="mt-1 text-3xl tracking-tightish text-slate-900 dark:text-slate-100"
              style={{ fontWeight: 650 }}
            >
              Patterns over time
            </h1>
            <p className="mt-1 text-sm text-slate-600 dark:text-slate-400">
              Built from nightly rollups in `daily_summaries` (refreshed hourly).
            </p>
          </div>
          <div className="flex flex-wrap gap-2">
            {([7, 30, 90] as const).map((d) => (
              <button
                key={d}
                type="button"
                onClick={() => setRange(d)}
                className={`rounded-full px-3 py-1.5 text-xs font-medium ${
                  range === d
                    ? "bg-tangerine-500 text-white"
                    : "bg-slate-100 text-slate-600 hover:bg-slate-200 dark:bg-slate-800 dark:text-slate-300 dark:hover:bg-slate-700"
                }`}
              >
                Last {d} days
              </button>
            ))}
          </div>
        </header>

        {error && (
          <div className="rounded-xl border border-red-200 bg-red-50 p-3 text-sm text-red-700 dark:border-red-900/50 dark:bg-red-950/40 dark:text-red-300">
            {error}
          </div>
        )}

        {loading || !overview ? (
          <div className="space-y-4">
            {Array.from({ length: 4 }).map((_, i) => (
              <div
                key={i}
                className="h-56 animate-pulse rounded-2xl bg-slate-100 dark:bg-slate-800"
              />
            ))}
          </div>
        ) : (
          <>
            <motion.section
              initial={{ opacity: 0, y: 6 }}
              animate={{ opacity: 1, y: 0 }}
              className="rounded-2xl border border-slate-100 bg-white p-5 shadow-soft dark:border-slate-800 dark:bg-slate-900"
            >
              <h2 className="text-lg font-semibold text-slate-900 dark:text-slate-100">
                Focus score trend
              </h2>
              <div className="mt-3 h-64">
                <ResponsiveContainer width="100%" height="100%">
                  <LineChart data={overview.focus_score_trend}>
                    <CartesianGrid strokeDasharray="3 3" stroke={pal.grid} />
                    <XAxis dataKey="date" tick={{ fontSize: 11, fill: pal.tick }} />
                    <YAxis domain={[0, 100]} width={32} tick={{ fill: pal.tick }} />
                    <Tooltip
                      contentStyle={{
                        backgroundColor: pal.tooltipBg,
                        border: `1px solid ${pal.tooltipBorder}`,
                      }}
                    />
                    <Line
                      type="monotone"
                      dataKey="score"
                      stroke="#F28500"
                      strokeWidth={2}
                      dot
                      animationDuration={600}
                    />
                  </LineChart>
                </ResponsiveContainer>
              </div>
            </motion.section>

            <div className="grid gap-6 md:grid-cols-2">
              <motion.section
                initial={{ opacity: 0, y: 6 }}
                animate={{ opacity: 1, y: 0 }}
                transition={{ delay: 0.05 }}
                className="rounded-2xl border border-slate-100 bg-white p-5 shadow-soft dark:border-slate-800 dark:bg-slate-900"
              >
                <h2 className="text-lg font-semibold text-slate-900 dark:text-slate-100">
                  Average focus streak
                </h2>
                <p className="text-xs text-slate-500 dark:text-slate-400">
                  Longest focus/work run per day
                </p>
                <div className="mt-3 h-56">
                  <ResponsiveContainer width="100%" height="100%">
                    <BarChart data={overview.avg_streak_trend}>
                      <CartesianGrid strokeDasharray="3 3" stroke={pal.grid} />
                      <XAxis dataKey="date" tick={{ fontSize: 10, fill: pal.tick }} />
                      <YAxis
                        tickFormatter={(v) => `${Math.round(v / 60)}m`}
                        width={36}
                        tick={{ fill: pal.tick }}
                      />
                      <Tooltip
                        formatter={(v) => formatDuration(Number(v))}
                        labelFormatter={(l) => l}
                        contentStyle={{
                          backgroundColor: pal.tooltipBg,
                          border: `1px solid ${pal.tooltipBorder}`,
                        }}
                      />
                      <Bar dataKey="seconds" fill="#F28500" radius={[4, 4, 0, 0]} />
                    </BarChart>
                  </ResponsiveContainer>
                </div>
              </motion.section>

              <motion.section
                initial={{ opacity: 0, y: 6 }}
                animate={{ opacity: 1, y: 0 }}
                transition={{ delay: 0.1 }}
                className="rounded-2xl border border-slate-100 bg-white p-5 shadow-soft dark:border-slate-800 dark:bg-slate-900"
              >
                <h2 className="text-lg font-semibold text-slate-900 dark:text-slate-100">
                  Total switches per day
                </h2>
                <div className="mt-3 h-56">
                  <ResponsiveContainer width="100%" height="100%">
                    <BarChart data={switchesData}>
                      <CartesianGrid strokeDasharray="3 3" stroke={pal.grid} />
                      <XAxis dataKey="date" tick={{ fontSize: 10, fill: pal.tick }} />
                      <YAxis width={32} tick={{ fill: pal.tick }} />
                      <Tooltip
                        contentStyle={{
                          backgroundColor: pal.tooltipBg,
                          border: `1px solid ${pal.tooltipBorder}`,
                        }}
                      />
                      <Bar dataKey="switches" radius={[4, 4, 0, 0]}>
                        {switchesData.map((e, i) => (
                          <Cell key={i} fill={e.fill} />
                        ))}
                      </Bar>
                    </BarChart>
                  </ResponsiveContainer>
                </div>
              </motion.section>
            </div>

            <motion.section
              initial={{ opacity: 0, y: 6 }}
              animate={{ opacity: 1, y: 0 }}
              transition={{ delay: 0.15 }}
              className="rounded-2xl border border-slate-100 bg-white p-5 shadow-soft dark:border-slate-800 dark:bg-slate-900"
            >
              <h2 className="text-lg font-semibold text-slate-900 dark:text-slate-100">
                Category mix
              </h2>
              <div className="mt-3 h-72">
                <ResponsiveContainer width="100%" height="100%">
                  <AreaChart data={overview.category_mix}>
                    <CartesianGrid strokeDasharray="3 3" stroke={pal.grid} />
                    <XAxis dataKey="date" tick={{ fontSize: 10, fill: pal.tick }} />
                    <YAxis tickFormatter={(v) => `${Math.round(v / 3600)}h`} tick={{ fill: pal.tick }} />
                    <Tooltip
                      formatter={(v) => formatDuration(Number(v))}
                      contentStyle={{
                        backgroundColor: pal.tooltipBg,
                        border: `1px solid ${pal.tooltipBorder}`,
                      }}
                    />
                    <Area
                      type="monotone"
                      stackId="1"
                      dataKey="focus_seconds"
                      name="Deep work"
                      stroke="#F28500"
                      fill="#FCE5C0"
                    />
                    <Area
                      type="monotone"
                      stackId="1"
                      dataKey="work_seconds"
                      name="Work"
                      stroke="#3B82F6"
                      fill="#BFDBFE"
                    />
                    <Area
                      type="monotone"
                      stackId="1"
                      dataKey="personal_seconds"
                      name="Personal"
                      stroke="#10B981"
                      fill="#BBF7D0"
                    />
                    <Area
                      type="monotone"
                      stackId="1"
                      dataKey="distracting_seconds"
                      name="Distracting"
                      stroke="#EF4444"
                      fill="#FECACA"
                    />
                  </AreaChart>
                </ResponsiveContainer>
              </div>
            </motion.section>

            <motion.section
              initial={{ opacity: 0, y: 6 }}
              animate={{ opacity: 1, y: 0 }}
              transition={{ delay: 0.2 }}
              className="grid gap-4 md:grid-cols-3"
            >
              <div className="rounded-2xl border border-slate-100 bg-white p-5 shadow-soft dark:border-slate-800 dark:bg-slate-900">
                <p className="text-xs uppercase tracking-wide text-slate-500 dark:text-slate-400">
                  Longest streak ever
                </p>
                <p className="mt-2 text-3xl font-bold text-slate-900 dark:text-slate-100">
                  {formatDuration(overview.personal_records.longest_streak_ever)}
                </p>
              </div>
              <div className="rounded-2xl border border-slate-100 bg-white p-5 shadow-soft dark:border-slate-800 dark:bg-slate-900">
                <p className="text-xs uppercase tracking-wide text-slate-500 dark:text-slate-400">
                  Best focus day
                </p>
                <p className="mt-2 text-3xl font-bold text-slate-900 dark:text-slate-100">
                  {overview.personal_records.best_day.focus_score}
                </p>
                <p className="text-xs text-slate-500 dark:text-slate-400">
                  {overview.personal_records.best_day.date || "—"}
                </p>
              </div>
              <div className="rounded-2xl border border-slate-100 bg-white p-5 shadow-soft dark:border-slate-800 dark:bg-slate-900">
                <p className="text-xs uppercase tracking-wide text-slate-500 dark:text-slate-400">
                  Days in view
                </p>
                <p className="mt-2 text-3xl font-bold text-slate-900 dark:text-slate-100">
                  {overview.daily_summaries.length}
                </p>
                <p className="text-xs text-slate-500 dark:text-slate-400">rollup rows loaded</p>
              </div>
            </motion.section>

            <motion.section
              initial={{ opacity: 0, y: 6 }}
              animate={{ opacity: 1, y: 0 }}
              transition={{ delay: 0.25 }}
              className="rounded-2xl border border-slate-100 bg-white p-5 shadow-soft dark:border-slate-800 dark:bg-slate-900"
            >
              <h2 className="text-lg font-semibold text-slate-900 dark:text-slate-100">
                Weekly heatmap (UTC)
              </h2>
              <p className="text-xs text-slate-500 dark:text-slate-400">
                Avg engagement · last 30 days of samples
              </p>
              <div className="mt-4 overflow-x-auto">
                <div
                  className="grid gap-px bg-slate-100 p-px dark:bg-slate-800"
                  style={{
                    gridTemplateColumns: `96px repeat(24, minmax(0, 1fr))`,
                  }}
                >
                  <div />
                  {Array.from({ length: 24 }).map((_, h) => (
                    <div
                      key={h}
                      className="text-center text-[10px] text-slate-500 dark:text-slate-400"
                    >
                      {h}
                    </div>
                  ))}
                  {DOW.flatMap((label, dow) => {
                    const row: ReactNode[] = [
                      <div
                        key={`l-${dow}`}
                        className="flex items-center bg-white px-2 text-xs text-slate-600 dark:bg-slate-900 dark:text-slate-300"
                      >
                        {label}
                      </div>,
                    ];
                    for (let hr = 0; hr < 24; hr += 1) {
                      const cell = heatmapMap.get(`${dow}-${hr}`);
                      const v = cell?.avg_engagement ?? 0;
                      const bg = `rgba(242,133,0,${(v / 100) * 0.9 + 0.05})`;
                      row.push(
                        <div
                          key={`c-${dow}-${hr}`}
                          title={`${label} ${hr}:00 · avg ${v}`}
                          className="h-6"
                          style={{ background: bg }}
                        />,
                      );
                    }
                    return row;
                  })}
                </div>
              </div>
            </motion.section>
          </>
        )}
      </div>
    </div>
  );
}
