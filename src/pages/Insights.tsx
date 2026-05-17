import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { formatDistanceToNow } from "date-fns";
import { motion } from "framer-motion";
import {
  AlertOctagon,
  AlertTriangle,
  Lightbulb,
  RefreshCw,
} from "lucide-react";
import { cn } from "../lib/utils";

interface Insight {
  id: number;
  created_at: number;
  title: string;
  body: string;
  severity: string;
  tag: string;
}

function iconFor(sev: string) {
  if (sev === "danger")
    return {
      Icon: AlertOctagon,
      color: "text-red-600 dark:text-red-400",
      bg: "bg-red-50 dark:bg-red-950/40",
    };
  if (sev === "warn")
    return {
      Icon: AlertTriangle,
      color: "text-amber-600 dark:text-amber-400",
      bg: "bg-amber-50 dark:bg-amber-950/40",
    };
  return {
    Icon: Lightbulb,
    color: "text-slate-700 dark:text-slate-300",
    bg: "bg-slate-50 dark:bg-slate-800",
  };
}

export function Insights() {
  const [rows, setRows] = useState<Insight[]>([]);
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const r = await invoke<Insight[]>("get_recent_insights", { limit: 40 });
      setRows(r);
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  const onRefresh = async () => {
    setBusy(true);
    try {
      await invoke("generate_insights_now");
      await load();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="min-h-full bg-gradient-to-b from-white to-tangerine-50/30 px-10 py-10 dark:from-slate-900 dark:to-tangerine-900/20">
      <div className="mx-auto max-w-3xl space-y-6">
        <header className="flex items-start justify-between gap-4">
          <div>
            <p className="text-xs font-medium uppercase tracking-[0.18em] text-tangerine-600 dark:text-tangerine-400">
              Insights
            </p>
            <h1
              className="mt-1 text-3xl tracking-tightish text-slate-900 dark:text-slate-100"
              style={{ fontWeight: 650 }}
            >
              Auto-generated takeaways
            </h1>
            <p className="mt-1 text-sm text-slate-600 dark:text-slate-400">
              Honest patterns from your real data — no cloud, no LLM.
            </p>
          </div>
          <button
            type="button"
            disabled={busy}
            onClick={onRefresh}
            className="inline-flex items-center gap-2 rounded-xl border border-slate-200 bg-white px-3 py-2 text-sm font-medium text-slate-700 shadow-sm hover:border-tangerine-200 hover:text-tangerine-700 disabled:opacity-50 dark:border-slate-700 dark:bg-slate-800 dark:text-slate-200 dark:hover:border-tangerine-600"
          >
            <RefreshCw className={cn("h-4 w-4", busy && "animate-spin")} />
            Refresh
          </button>
        </header>

        {error && (
          <div className="rounded-xl border border-red-200 bg-red-50 p-3 text-sm text-red-700 dark:border-red-900/50 dark:bg-red-950/40 dark:text-red-300">
            {error}
          </div>
        )}

        {loading ? (
          <div className="space-y-3">
            {Array.from({ length: 4 }).map((_, i) => (
              <div key={i} className="h-28 animate-pulse rounded-2xl bg-slate-100 dark:bg-slate-800" />
            ))}
          </div>
        ) : rows.length === 0 ? (
          <div className="rounded-2xl border border-slate-100 bg-white p-10 text-center text-slate-500 shadow-soft dark:border-slate-800 dark:bg-slate-900 dark:text-slate-400">
            Working on it. Insights appear after a few hours of tracking.
          </div>
        ) : (
          <ul className="space-y-4">
            {rows.map((ins, idx) => {
              const { Icon, color, bg } = iconFor(ins.severity);
              return (
                <motion.li
                  key={ins.id}
                  initial={{ opacity: 0, y: 6 }}
                  animate={{ opacity: 1, y: 0 }}
                  transition={{ delay: idx * 0.04 }}
                  className="rounded-2xl border border-slate-100 bg-white p-5 shadow-soft dark:border-slate-800 dark:bg-slate-900"
                >
                  <div className="flex items-start gap-3">
                    <div className={`rounded-xl p-2 ${bg}`}>
                      <Icon className={`h-5 w-5 ${color}`} />
                    </div>
                    <div className="flex-1 space-y-2">
                      <h2 className="text-lg font-semibold text-slate-900 dark:text-slate-100">
                        {ins.title}
                      </h2>
                      <p className="text-sm leading-relaxed text-slate-600 dark:text-slate-400">
                        {ins.body}
                      </p>
                      {ins.tag ? (
                        <span className="inline-block rounded-md bg-slate-100 px-2 py-0.5 font-mono text-[10px] text-slate-500 dark:bg-slate-800 dark:text-slate-400">
                          {ins.tag}
                        </span>
                      ) : null}
                      <p className="text-[11px] text-slate-400 dark:text-slate-500">
                        {formatDistanceToNow(new Date(ins.created_at), {
                          addSuffix: true,
                        })}
                      </p>
                    </div>
                  </div>
                </motion.li>
              );
            })}
          </ul>
        )}
      </div>
    </div>
  );
}
