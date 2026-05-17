import { useCallback, useEffect, useMemo, useState } from "react";
import { useSearchParams } from "react-router-dom";
import { invoke } from "@tauri-apps/api/core";
import { motion, AnimatePresence } from "framer-motion";
import {
  CalendarDays,
  ChevronLeft,
  ChevronRight,
  Download,
  Search,
} from "lucide-react";
import {
  endOfDay,
  format,
  startOfDay,
  subDays,
} from "date-fns";
import {
  CartesianGrid,
  Line,
  LineChart,
  ResponsiveContainer,
  Tooltip,
  XAxis,
  YAxis,
} from "recharts";
import { formatDuration } from "../lib/formatters";
import { cn } from "../lib/utils";
import { chartPalette, useTheme } from "../contexts/ThemeProvider";

type DatePreset = "today" | "yesterday" | "week" | "custom";

interface ActivityFilters {
  date_start?: number;
  date_end?: number;
  search?: string;
  category_ids?: number[];
  min_duration_seconds?: number;
  limit?: number;
  offset?: number;
  /** `"event"` | `"app"` | `"domain"` — omit or `"event"` for per-event rows */
  group_by?: string;
}

interface ActivityEventWithCategory {
  id: number;
  app_name: string;
  window_title: string | null;
  browser_url: string | null;
  browser_domain: string | null;
  category_id: number | null;
  started_at: number;
  ended_at: number | null;
  duration_seconds: number | null;
  category_name: string | null;
  category_color: string | null;
  avg_engagement: number;
}

interface Category {
  id: number;
  name: string;
  productivity_level: string;
  color: string | null;
  is_default: boolean;
}

interface EngagementSample {
  id: number;
  activity_event_id: number;
  sampled_at: number;
  mouse_clicks: number;
  key_presses: number;
  mouse_distance_pixels: number;
  scroll_events: number;
  is_idle: boolean;
  engagement_score: number;
}

interface EventEngagement {
  samples: EngagementSample[];
  avg_score: number;
  total_seconds_active: number;
}

const PAGE_SIZE = 50;
const MIN_DURATION_OPTIONS: { label: string; seconds: number }[] = [
  { label: "Any", seconds: 0 },
  { label: "10s+", seconds: 10 },
  { label: "30s+", seconds: 30 },
  { label: "1m+", seconds: 60 },
  { label: "5m+", seconds: 300 },
];

function toUtcMs(d: Date): number {
  return d.getTime();
}

function presetRange(
  preset: DatePreset,
  customStart: string,
  customEnd: string,
): { start: number; end: number } {
  const now = new Date();
  if (preset === "today") {
    return { start: toUtcMs(startOfDay(now)), end: toUtcMs(endOfDay(now)) };
  }
  if (preset === "yesterday") {
    const y = subDays(now, 1);
    return { start: toUtcMs(startOfDay(y)), end: toUtcMs(endOfDay(y)) };
  }
  if (preset === "week") {
    return {
      start: toUtcMs(startOfDay(subDays(now, 6))),
      end: toUtcMs(endOfDay(now)),
    };
  }
  const s = customStart ? new Date(`${customStart}T00:00:00`) : startOfDay(now);
  const e = customEnd ? new Date(`${customEnd}T23:59:59.999`) : endOfDay(now);
  return { start: toUtcMs(s), end: toUtcMs(e) };
}

export function Activity() {
  const [preset, setPreset] = useState<DatePreset>("today");
  const [customStart, setCustomStart] = useState(
    format(subDays(new Date(), 7), "yyyy-MM-dd"),
  );
  const [customEnd, setCustomEnd] = useState(format(new Date(), "yyyy-MM-dd"));
  const [search, setSearch] = useState("");
  const [debouncedSearch, setDebouncedSearch] = useState("");
  const [categories, setCategories] = useState<Category[]>([]);
  const [selectedCats, setSelectedCats] = useState<number[]>([]);
  const [minDur, setMinDur] = useState(0);
  const [page, setPage] = useState(0);
  const [rows, setRows] = useState<ActivityEventWithCategory[]>([]);
  const [total, setTotal] = useState(0);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [expandedId, setExpandedId] = useState<number | null>(null);
  const [eventEngagement, setEventEngagement] = useState<EventEngagement | null>(
    null,
  );
  const [engLoading, setEngLoading] = useState(false);
  const [popover, setPopover] = useState<{
    event: ActivityEventWithCategory;
  } | null>(null);
  const [retro, setRetro] = useState(true);
  const [groupBy, setGroupBy] = useState<"event" | "app" | "domain">("event");
  const [aggregateDetails, setAggregateDetails] = useState<
    ActivityEventWithCategory[]
  >([]);
  const [aggLoading, setAggLoading] = useState(false);

  const [searchParams, setSearchParams] = useSearchParams();
  useEffect(() => {
    const q = searchParams.toString();
    if (!q) return;
    const view = searchParams.get("view");
    const filter = searchParams.get("filter");
    if (view === "app" || view === "domain") {
      setGroupBy(view);
    }
    if (filter != null && filter.length > 0) {
      setSearch(filter);
    }
    setSearchParams({}, { replace: true });
  }, [searchParams, setSearchParams]);

  useEffect(() => {
    const t = window.setTimeout(() => setDebouncedSearch(search.trim()), 300);
    return () => window.clearTimeout(t);
  }, [search]);

  useEffect(() => {
    invoke<Category[]>("list_categories")
      .then(setCategories)
      .catch(() => {});
  }, []);

  const { start, end } = useMemo(
    () => presetRange(preset, customStart, customEnd),
    [preset, customStart, customEnd],
  );

  const buildFilters = useCallback(
    (pageIndex: number): ActivityFilters => ({
      date_start: start,
      date_end: end,
      search: debouncedSearch || undefined,
      category_ids: selectedCats.length ? selectedCats : undefined,
      min_duration_seconds: minDur || undefined,
      limit: PAGE_SIZE,
      offset: pageIndex * PAGE_SIZE,
      group_by: groupBy === "event" ? undefined : groupBy,
    }),
    [start, end, debouncedSearch, selectedCats, minDur, groupBy],
  );

  const refresh = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const f = buildFilters(page);
      const [list, count] = await Promise.all([
        invoke<ActivityEventWithCategory[]>("get_activity_events", { filters: f }),
        invoke<number>("get_activity_event_count", { filters: f }),
      ]);
      setRows(list);
      setTotal(typeof count === "bigint" ? Number(count) : count);
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  }, [buildFilters, page]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  useEffect(() => {
    setPage(0);
  }, [start, end, debouncedSearch, selectedCats.join(","), minDur, groupBy]);

  useEffect(() => {
    setExpandedId(null);
  }, [groupBy]);

  useEffect(() => {
    if (expandedId == null || groupBy !== "event") {
      setEventEngagement(null);
      return;
    }
    setEngLoading(true);
    invoke<EventEngagement>("get_engagement_for_event", { event_id: expandedId })
      .then(setEventEngagement)
      .catch(() => setEventEngagement(null))
      .finally(() => setEngLoading(false));
  }, [expandedId, groupBy]);

  useEffect(() => {
    if (expandedId == null || groupBy === "event") {
      setAggregateDetails([]);
      return;
    }
    const row = rows.find((r) => r.id === expandedId);
    if (!row) {
      setAggregateDetails([]);
      return;
    }
    setAggLoading(true);
    invoke<ActivityEventWithCategory[]>("get_aggregate_event_details", {
      name: row.app_name,
      kind: groupBy,
      filters: {
        date_start: start,
        date_end: end,
        search: debouncedSearch || undefined,
        category_ids: selectedCats.length ? selectedCats : undefined,
        min_duration_seconds: minDur || undefined,
      },
    })
      .then(setAggregateDetails)
      .catch(() => setAggregateDetails([]))
      .finally(() => setAggLoading(false));
  }, [expandedId, groupBy, start, end, debouncedSearch, selectedCats.join(","), minDur, page, rows]);

  const totalPages = Math.max(1, Math.ceil(total / PAGE_SIZE));

  const toggleCat = (id: number) => {
    setSelectedCats((prev) =>
      prev.includes(id) ? prev.filter((c) => c !== id) : [...prev, id],
    );
  };

  const exportCsv = () => {
    const header = [
      "id",
      "started_at",
      "ended_at",
      "duration_seconds",
      "app_name",
      "window_title",
      "browser_url",
      "browser_domain",
      "category",
      "avg_engagement",
    ];
    const lines = rows.map((r) =>
      [
        r.id,
        r.started_at,
        r.ended_at ?? "",
        r.duration_seconds ?? "",
        csvEscape(r.app_name),
        csvEscape(r.window_title ?? ""),
        csvEscape(r.browser_url ?? ""),
        csvEscape(r.browser_domain ?? ""),
        csvEscape(r.category_name ?? ""),
        r.avg_engagement,
      ].join(","),
    );
    const blob = new Blob([[header.join(","), ...lines].join("\n")], {
      type: "text/csv;charset=utf-8",
    });
    const url = URL.createObjectURL(blob);
    const a = document.createElement("a");
    a.href = url;
    a.download = `activity-${groupBy}-${format(new Date(), "yyyy-MM-dd")}.csv`;
    a.click();
    URL.revokeObjectURL(url);
  };

  return (
    <div className="min-h-full bg-gradient-to-b from-white to-tangerine-50/30 px-10 py-10 dark:from-slate-900 dark:to-tangerine-900/20">
      <div className="mx-auto max-w-6xl space-y-6">
        <header className="flex flex-col gap-4 md:flex-row md:items-center md:justify-between">
          <div>
            <p className="text-xs font-medium uppercase tracking-[0.18em] text-tangerine-600">
              Timeline
            </p>
            <h1
              className="mt-1 text-3xl tracking-tightish text-slate-900 dark:text-slate-100"
              style={{ fontWeight: 650 }}
            >
              Activity
            </h1>
            <p className="mt-1 text-sm text-slate-600 dark:text-slate-400">
              Searchable history of every focus interval we recorded.
            </p>
          </div>
          <button
            type="button"
            onClick={exportCsv}
            className="inline-flex items-center gap-2 self-start rounded-xl border border-slate-200 bg-white px-4 py-2 text-sm font-medium text-slate-700 shadow-sm transition hover:border-tangerine-200 hover:text-tangerine-700 dark:border-slate-700 dark:bg-slate-800 dark:text-slate-200 dark:hover:border-tangerine-600"
          >
            <Download className="h-4 w-4" />
            Export CSV
          </button>
        </header>

        <section className="rounded-2xl border border-slate-100 bg-white p-5 shadow-soft dark:border-slate-800 dark:bg-slate-900">
          <div className="flex flex-wrap items-center gap-2">
            {(
              [
                ["today", "Today"],
                ["yesterday", "Yesterday"],
                ["week", "This week"],
                ["custom", "Custom"],
              ] as const
            ).map(([key, label]) => (
              <button
                key={key}
                type="button"
                onClick={() => setPreset(key)}
                className={cn(
                  "rounded-full px-3 py-1.5 text-xs font-medium transition",
                  preset === key
                    ? "bg-tangerine-500 text-white"
                    : "bg-slate-100 text-slate-600 hover:bg-slate-200 dark:bg-slate-800 dark:text-slate-300 dark:hover:bg-slate-700",
                )}
              >
                {label}
              </button>
            ))}
          </div>
          {preset === "custom" && (
            <div className="mt-3 flex flex-wrap items-center gap-2 text-sm">
              <CalendarDays className="h-4 w-4 text-slate-400" />
              <input
                type="date"
                value={customStart}
                onChange={(e) => setCustomStart(e.target.value)}
                className="rounded-lg border border-slate-200 px-2 py-1"
              />
              <span className="text-slate-400">to</span>
              <input
                type="date"
                value={customEnd}
                onChange={(e) => setCustomEnd(e.target.value)}
                className="rounded-lg border border-slate-200 px-2 py-1"
              />
            </div>
          )}

          <div className="mt-4 flex flex-wrap items-center gap-2">
            <span className="text-xs font-medium uppercase tracking-wide text-slate-500">
              View by:
            </span>
            {(
              [
                ["event", "Events"],
                ["app", "Apps"],
                ["domain", "Websites"],
              ] as const
            ).map(([key, label]) => (
              <button
                key={key}
                type="button"
                onClick={() => setGroupBy(key)}
                className={cn(
                  "rounded-full px-3 py-1.5 text-xs font-medium transition",
                  groupBy === key
                    ? "bg-tangerine-500 text-white"
                    : "bg-slate-100 text-slate-600 hover:bg-slate-200 dark:bg-slate-800 dark:text-slate-300 dark:hover:bg-slate-700",
                )}
              >
                {label}
              </button>
            ))}
          </div>

          <div className="mt-4 grid gap-4 md:grid-cols-3">
            <div className="relative md:col-span-1">
              <Search className="pointer-events-none absolute left-3 top-2.5 h-4 w-4 text-slate-400" />
              <input
                value={search}
                onChange={(e) => setSearch(e.target.value)}
                placeholder={
                  groupBy === "domain"
                    ? "Filter by website or domain…"
                    : "Search app or window title…"
                }
                className="w-full rounded-xl border border-slate-200 py-2 pl-9 pr-3 text-sm outline-none ring-tangerine-200 focus:ring-2"
              />
            </div>
            <div>
              <p className="mb-1 text-xs font-medium uppercase tracking-wide text-slate-500">
                Categories
              </p>
              <div className="flex max-h-28 flex-wrap gap-1 overflow-y-auto rounded-xl border border-slate-100 bg-slate-50/80 p-2">
                {categories.map((c) => (
                  <button
                    key={c.id}
                    type="button"
                    onClick={() => toggleCat(c.id)}
                    className={cn(
                      "rounded-full px-2 py-0.5 text-[11px] font-medium transition",
                      selectedCats.includes(c.id)
                        ? "bg-tangerine-500 text-white"
                        : "bg-white text-slate-600 ring-1 ring-slate-200",
                    )}
                  >
                    {c.name}
                  </button>
                ))}
              </div>
            </div>
            <div>
              <p className="mb-1 text-xs font-medium uppercase tracking-wide text-slate-500">
                Min duration
              </p>
              <select
                value={minDur}
                onChange={(e) => setMinDur(Number(e.target.value))}
                className="w-full rounded-xl border border-slate-200 bg-white px-3 py-2 text-sm"
              >
                {MIN_DURATION_OPTIONS.map((o) => (
                  <option key={o.seconds} value={o.seconds}>
                    {o.label}
                  </option>
                ))}
              </select>
            </div>
          </div>
        </section>

        {error && (
          <div className="rounded-xl border border-red-200 bg-red-50 p-3 text-sm text-red-700">
            {error}
          </div>
        )}

        <section className="overflow-hidden rounded-2xl border border-slate-100 bg-white shadow-soft dark:border-slate-800 dark:bg-slate-900">
          <div className="flex items-center justify-between border-b border-slate-100 px-4 py-3 dark:border-slate-800">
            <p className="text-sm text-slate-600 dark:text-slate-400">
              <span className="font-semibold text-slate-900 dark:text-slate-100">{total}</span>{" "}
              {groupBy === "event" ? "matching events" : "matching groups"}
            </p>
            <div className="flex items-center gap-2">
              <button
                type="button"
                disabled={page <= 0}
                onClick={() => setPage((p) => Math.max(0, p - 1))}
                className="rounded-lg border border-slate-200 p-1 disabled:opacity-40"
              >
                <ChevronLeft className="h-4 w-4" />
              </button>
              <span className="text-xs text-slate-500">
                Page {page + 1} / {totalPages}
              </span>
              <button
                type="button"
                disabled={page + 1 >= totalPages}
                onClick={() => setPage((p) => p + 1)}
                className="rounded-lg border border-slate-200 p-1 disabled:opacity-40"
              >
                <ChevronRight className="h-4 w-4" />
              </button>
            </div>
          </div>

          <div className="overflow-x-auto">
            <table className="min-w-full text-left text-sm">
              <thead className="bg-slate-50 text-xs uppercase tracking-wide text-slate-500 dark:bg-slate-800/80 dark:text-slate-400">
                <tr>
                  <th className="px-4 py-2">
                    {groupBy === "domain" ? "Latest" : "Time"}
                  </th>
                  <th className="px-4 py-2">
                    {groupBy === "domain" ? "Website" : "App"}
                  </th>
                  {groupBy !== "domain" && (
                    <th className="px-4 py-2">Window</th>
                  )}
                  <th className="px-4 py-2">Duration</th>
                  <th className="px-4 py-2">Engagement</th>
                  <th className="px-4 py-2">Category</th>
                </tr>
              </thead>
              <tbody>
                {loading ? (
                  Array.from({ length: 8 }).map((_, i) => (
                    <tr key={i} className="border-t border-slate-100">
                      <td
                        colSpan={groupBy === "domain" ? 5 : 6}
                        className="px-4 py-3"
                      >
                        <div className="h-4 animate-pulse rounded bg-slate-100" />
                      </td>
                    </tr>
                  ))
                ) : rows.length === 0 ? (
                  <tr>
                    <td
                      colSpan={groupBy === "domain" ? 5 : 6}
                      className="px-4 py-10 text-center text-slate-500"
                    >
                      {groupBy === "domain" &&
                      !debouncedSearch &&
                      selectedCats.length === 0 &&
                      minDur === 0 ? (
                        <div className="mx-auto max-w-sm">
                          <p className="text-base font-medium text-slate-700 dark:text-slate-300">
                            No browsing tracked yet
                          </p>
                          <p className="mt-2 text-sm text-slate-500 dark:text-slate-400">
                            Open sites in Chrome with the tab tracker extension
                            enabled. Visits will show here grouped by website.
                          </p>
                        </div>
                      ) : (
                        "No events match these filters."
                      )}
                    </td>
                  </tr>
                ) : (
                  rows.map((r) => (
                    <ActivityRow
                      key={`${groupBy}-${r.id}`}
                      row={r}
                      groupBy={groupBy}
                      expanded={expandedId === r.id}
                      onToggle={() =>
                        setExpandedId((id) => (id === r.id ? null : r.id))
                      }
                      onBadgeClick={() => setPopover({ event: r })}
                      engagement={expandedId === r.id ? eventEngagement : null}
                      engLoading={expandedId === r.id && engLoading}
                      aggregateDetails={
                        expandedId === r.id ? aggregateDetails : []
                      }
                      aggLoading={expandedId === r.id && aggLoading}
                    />
                  ))
                )}
              </tbody>
            </table>
          </div>
        </section>
      </div>

      {popover && groupBy !== "domain" && (
        <CategoryPopover
          categories={categories}
          event={popover.event}
          retro={retro}
          setRetro={setRetro}
          onClose={() => setPopover(null)}
          onApply={async (categoryId: number) => {
            await invoke("recategorize_app", {
              app_name: popover.event.app_name,
              category_id: categoryId,
              retroactive: retro,
            });
            setPopover(null);
            await refresh();
          }}
        />
      )}
    </div>
  );
}

function csvEscape(s: string): string {
  if (s.includes(",") || s.includes('"') || s.includes("\n")) {
    return `"${s.replace(/"/g, '""')}"`;
  }
  return s;
}

function ActivityRow({
  row,
  groupBy,
  expanded,
  onToggle,
  onBadgeClick,
  engagement,
  engLoading,
  aggregateDetails,
  aggLoading,
}: {
  row: ActivityEventWithCategory;
  groupBy: "event" | "app" | "domain";
  expanded: boolean;
  onToggle: () => void;
  onBadgeClick: () => void;
  engagement: EventEngagement | null;
  engLoading: boolean;
  aggregateDetails: ActivityEventWithCategory[];
  aggLoading: boolean;
}) {
  const { resolvedTheme } = useTheme();
  const pal = chartPalette(resolvedTheme);
  const started = new Date(row.started_at);
  const dur =
    row.duration_seconds ??
    Math.max(0, Math.round((Date.now() - row.started_at) / 1000));
  const dot = row.category_color ?? "#94a3b8";
  const isAggregate = groupBy !== "event";
  const domainForIcon =
    groupBy === "domain" ? row.browser_domain ?? row.app_name : null;
  const faviconUrl = domainForIcon
    ? `https://www.google.com/s2/favicons?domain=${encodeURIComponent(domainForIcon)}&sz=32`
    : null;
  const [faviconBroken, setFaviconBroken] = useState(false);
  useEffect(() => {
    setFaviconBroken(false);
  }, [faviconUrl]);

  const letterFallback = (domainForIcon ?? row.app_name ?? "?")
    .charAt(0)
    .toUpperCase();

  const showWindowCol = groupBy !== "domain";
  const colSpan = showWindowCol ? 6 : 5;

  return (
    <>
      <tr
        className="cursor-pointer border-t border-slate-100 hover:bg-tangerine-50/40 dark:border-slate-800 dark:hover:bg-tangerine-950/30"
        onClick={onToggle}
      >
        <td className="whitespace-nowrap px-4 py-2 text-slate-700">
          {format(started, "h:mm a")}
        </td>
        <td className="px-4 py-2 font-medium text-slate-900 dark:text-slate-100">
          <span className="inline-flex items-center gap-2">
            {faviconUrl && !faviconBroken ? (
              <img
                src={faviconUrl}
                alt=""
                className="h-4 w-4 shrink-0 rounded-sm"
                loading="lazy"
                referrerPolicy="no-referrer"
                onError={() => setFaviconBroken(true)}
              />
            ) : domainForIcon ? (
              <span
                className="flex h-4 w-4 shrink-0 items-center justify-center rounded-sm bg-slate-200 text-[10px] font-semibold uppercase text-slate-600 dark:bg-slate-700 dark:text-slate-300"
                aria-hidden
              >
                {letterFallback}
              </span>
            ) : null}
            {row.app_name}
          </span>
        </td>
        {showWindowCol && (
          <td
            className="max-w-xs truncate px-4 py-2 text-slate-600 dark:text-slate-400"
            title={row.window_title ?? ""}
          >
            {row.window_title ?? "—"}
          </td>
        )}
        <td className="whitespace-nowrap px-4 py-2 text-slate-700">
          {formatDuration(dur)}
        </td>
        <td className="px-4 py-2">
          <div className="h-1.5 w-24 overflow-hidden rounded-full bg-slate-100 dark:bg-slate-800">
            <div
              className="h-full rounded-full bg-tangerine-500"
              style={{ width: `${row.avg_engagement}%` }}
            />
          </div>
        </td>
        <td className="px-4 py-2" onClick={(e) => e.stopPropagation()}>
          {groupBy === "domain" ? (
            <span className="text-xs text-slate-400">—</span>
          ) : (
            <button
              type="button"
              onClick={onBadgeClick}
              className="inline-flex items-center gap-1 rounded-full bg-slate-50 px-2 py-0.5 text-[11px] font-medium text-slate-700 ring-1 ring-slate-200 hover:ring-tangerine-300 dark:bg-slate-800 dark:text-slate-200 dark:ring-slate-700"
            >
              <span
                className="inline-block h-1.5 w-1.5 rounded-full"
                style={{ background: dot }}
              />
              {row.category_name ?? "Uncategorized"}
            </button>
          )}
        </td>
      </tr>
      <AnimatePresence>
        {expanded && (
          <tr className="border-t border-slate-100 bg-slate-50/60 dark:border-slate-800 dark:bg-slate-800/40">
            <td colSpan={colSpan} className="px-4 py-4">
              <motion.div
                initial={{ opacity: 0, height: 0 }}
                animate={{ opacity: 1, height: "auto" }}
                exit={{ opacity: 0, height: 0 }}
                className="space-y-3"
              >
                {!isAggregate && (
                  <>
                    <p className="text-sm text-slate-700">
                      <span className="font-semibold">Title:</span>{" "}
                      {row.window_title ?? "—"}
                    </p>
                    {row.browser_url && (
                      <p className="text-sm text-slate-700 break-all">
                        <span className="font-semibold">URL:</span>{" "}
                        {row.browser_url}
                      </p>
                    )}
                    {engLoading ? (
                      <div className="h-32 animate-pulse rounded-xl bg-slate-100" />
                    ) : engagement && engagement.samples.length > 0 ? (
                      <div className="h-40">
                        <ResponsiveContainer width="100%" height="100%">
                          <LineChart data={engagement.samples}>
                            <CartesianGrid
                              strokeDasharray="3 3"
                              stroke={pal.grid}
                            />
                            <XAxis
                              dataKey="sampled_at"
                              tickFormatter={(t) =>
                                format(new Date(t as number), "HH:mm:ss")
                              }
                              hide
                            />
                            <YAxis
                              domain={[0, 100]}
                              width={28}
                              tick={{ fill: pal.tick }}
                            />
                            <Tooltip
                              labelFormatter={(t) =>
                                format(new Date(t as number), "MMM d, h:mm:ss a")
                              }
                              contentStyle={{
                                backgroundColor: pal.tooltipBg,
                                border: `1px solid ${pal.tooltipBorder}`,
                              }}
                            />
                            <Line
                              type="monotone"
                              dataKey="engagement_score"
                              stroke="#F28500"
                              strokeWidth={2}
                              dot={false}
                              animationDuration={500}
                            />
                          </LineChart>
                        </ResponsiveContainer>
                      </div>
                    ) : (
                      <p className="text-xs text-slate-500">
                        No engagement samples for this interval.
                      </p>
                    )}
                  </>
                )}
                {isAggregate && (
                  <AggregateEventsSubtable
                    loading={aggLoading}
                    events={aggregateDetails}
                  />
                )}
              </motion.div>
            </td>
          </tr>
        )}
      </AnimatePresence>
    </>
  );
}

function AggregateEventsSubtable({
  loading,
  events,
}: {
  loading: boolean;
  events: ActivityEventWithCategory[];
}) {
  if (loading) {
    return (
      <div className="h-24 animate-pulse rounded-xl bg-slate-100" />
    );
  }
  if (events.length === 0) {
    return (
      <p className="text-xs text-slate-500">No underlying events found.</p>
    );
  }
  return (
    <div className="overflow-x-auto rounded-xl border border-slate-200 bg-white dark:border-slate-700 dark:bg-slate-900">
      <table className="w-full text-left text-xs">
        <thead className="border-b border-slate-100 bg-slate-50/80 text-[10px] uppercase tracking-wide text-slate-500 dark:border-slate-800 dark:bg-slate-800/80 dark:text-slate-400">
          <tr>
            <th className="px-3 py-2">Time</th>
            <th className="px-3 py-2">Window / title</th>
            <th className="px-3 py-2">Duration</th>
            <th className="px-3 py-2">Engagement</th>
          </tr>
        </thead>
        <tbody>
          {events.map((ev) => {
            const t = new Date(ev.started_at);
            const d =
              ev.duration_seconds ??
              Math.max(
                0,
                Math.round((Date.now() - ev.started_at) / 1000),
              );
            const title =
              ev.window_title?.trim() ||
              ev.browser_url?.trim() ||
              "—";
            return (
              <tr key={ev.id} className="border-t border-slate-100 dark:border-slate-800">
                <td className="whitespace-nowrap px-3 py-2 text-slate-700 dark:text-slate-300">
                  {format(t, "h:mm a")}
                </td>
                <td
                  className="max-w-xs truncate px-3 py-2 text-slate-600 dark:text-slate-400"
                  title={title}
                >
                  {title}
                </td>
                <td className="whitespace-nowrap px-3 py-2 text-slate-700 dark:text-slate-300">
                  {formatDuration(d)}
                </td>
                <td className="px-3 py-2">
                  <div className="h-1.5 w-20 overflow-hidden rounded-full bg-slate-100 dark:bg-slate-800">
                    <div
                      className="h-full rounded-full bg-tangerine-500"
                      style={{ width: `${ev.avg_engagement}%` }}
                    />
                  </div>
                </td>
              </tr>
            );
          })}
        </tbody>
      </table>
    </div>
  );
}

function CategoryPopover({
  categories,
  event,
  retro,
  setRetro,
  onClose,
  onApply,
}: {
  categories: Category[];
  event: ActivityEventWithCategory;
  retro: boolean;
  setRetro: (v: boolean) => void;
  onClose: () => void;
  onApply: (categoryId: number) => Promise<void>;
}) {
  const [busy, setBusy] = useState(false);
  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/30 p-4">
      <motion.div
        initial={{ opacity: 0, y: 6 }}
        animate={{ opacity: 1, y: 0 }}
        className="w-full max-w-md rounded-2xl border border-slate-100 bg-white p-5 shadow-soft dark:border-slate-800 dark:bg-slate-900"
      >
        <p className="text-sm font-semibold text-slate-900 dark:text-slate-100">
          Categorize “{event.app_name}”
        </p>
        <p className="mt-1 text-xs text-slate-500">
          Pick a category. Optionally rewrite history for this app.
        </p>
        <div className="mt-3 max-h-48 space-y-1 overflow-y-auto">
          {categories.map((c) => (
            <button
              key={c.id}
              type="button"
              disabled={busy}
              onClick={async () => {
                setBusy(true);
                try {
                  await onApply(c.id);
                } finally {
                  setBusy(false);
                }
              }}
              className="flex w-full items-center justify-between rounded-lg px-2 py-1.5 text-left text-sm hover:bg-slate-50"
            >
              <span className="flex items-center gap-2">
                <span
                  className="inline-block h-2 w-2 rounded-full"
                  style={{ background: c.color ?? "#94a3b8" }}
                />
                {c.name}
              </span>
            </button>
          ))}
        </div>
        <label className="mt-3 flex items-center gap-2 text-xs text-slate-600">
          <input
            type="checkbox"
            checked={retro}
            onChange={(e) => setRetro(e.target.checked)}
          />
          Apply to all past events for this app
        </label>
        <div className="mt-4 flex justify-end gap-2">
          <button
            type="button"
            onClick={onClose}
            className="rounded-lg px-3 py-1.5 text-sm text-slate-600 hover:bg-slate-100"
          >
            Cancel
          </button>
        </div>
      </motion.div>
    </div>
  );
}
