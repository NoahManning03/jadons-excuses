import { format, formatDistanceToNowStrict } from "date-fns";

export function formatTimestamp(date: Date | number): string {
  return format(date, "MMM d, yyyy · h:mm a");
}

/**
 * Human-readable duration string.
 *
 *   formatDuration(12)       // "12s"
 *   formatDuration(45 * 60)  // "45m"
 *   formatDuration(2.5 * 60 * 60) // "2h 30m"
 *
 * Notes:
 *   - Sub-minute → seconds.
 *   - Sub-hour   → bare minutes ("45m"), no leading "0h" noise.
 *   - ≥1 hour    → "Xh Ym". We deliberately drop seconds at this scale
 *     because they read as visual noise on a dashboard hero stat.
 *   - Negative values are clamped to 0 — the tracker can briefly emit
 *     a negative `(now - started_at)/1000` if the system clock skews.
 */
export function formatDuration(seconds: number): string {
  const s = Math.max(0, Math.round(seconds));
  if (s < 60) return `${s}s`;
  const minutes = Math.floor(s / 60);
  if (minutes < 60) return `${minutes}m`;
  const hours = Math.floor(minutes / 60);
  const min = minutes % 60;
  return `${hours}h ${min}m`;
}

/** Live dashboard counters — always shows seconds so values visibly tick. */
export function formatDurationLive(seconds: number): string {
  const s = Math.max(0, Math.floor(seconds));
  if (s < 60) return `${s}s`;
  const secs = s % 60;
  const minutes = Math.floor((s % 3600) / 60);
  const hours = Math.floor(s / 3600);
  if (hours > 0) {
    return `${hours}h ${minutes}m ${secs}s`;
  }
  return `${minutes}m ${secs}s`;
}

/**
 * Compact duration variant for tight UI slots (rows in the top-apps
 * list, tooltip labels). Trades the space-separated layout for a
 * shorter "1h2m" form.
 */
export function formatDurationShort(seconds: number): string {
  const s = Math.max(0, Math.round(seconds));
  if (s < 60) return `${s}s`;
  const minutes = Math.floor(s / 60);
  if (minutes < 60) return `${minutes}m`;
  const hours = Math.floor(minutes / 60);
  const min = minutes % 60;
  return min === 0 ? `${hours}h` : `${hours}h${min}m`;
}

/** "73%" — null/undefined-safe; clamps to 0..100. */
export function formatPercent(num: number): string {
  if (!Number.isFinite(num)) return "0%";
  const clamped = Math.max(0, Math.min(100, Math.round(num)));
  return `${clamped}%`;
}

export function formatRelative(date: Date | number): string {
  return `${formatDistanceToNowStrict(date)} ago`;
}
