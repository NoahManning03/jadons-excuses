export interface ActivityEvent {
  id: string;
  startedAt: number;
  endedAt: number;
  source: "window" | "browser";
  appName: string;
  windowTitle?: string;
  url?: string;
  domain?: string;
  category?: string;
}

export interface DailySummary {
  date: string;
  totalSeconds: number;
  focusedSeconds: number;
  fragmentedSeconds: number;
  topApps: Array<{ appName: string; seconds: number }>;
}

export interface Insight {
  id: string;
  createdAt: number;
  title: string;
  body: string;
  severity: "info" | "warn" | "danger";
}
