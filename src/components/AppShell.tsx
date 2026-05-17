import { NavLink, Outlet } from "react-router-dom";
import {
  Activity as ActivityIcon,
  BarChart3,
  LayoutDashboard,
  Lightbulb,
  Settings as SettingsIcon,
} from "lucide-react";
import { cn } from "../lib/utils";

const NAV = [
  { to: "/dashboard", label: "Dashboard", icon: LayoutDashboard },
  { to: "/activity", label: "Activity", icon: ActivityIcon },
  { to: "/trends", label: "Trends", icon: BarChart3 },
  { to: "/insights", label: "Insights", icon: Lightbulb },
  { to: "/settings", label: "Settings", icon: SettingsIcon },
];

export function AppShell() {
  return (
    <div className="flex h-full min-h-screen w-full bg-background dark:bg-slate-950">
      <aside className="flex w-60 shrink-0 flex-col border-r border-slate-100 bg-white/70 px-4 py-6 backdrop-blur dark:border-slate-800 dark:bg-slate-900/80">
        <div className="mb-8 px-2">
          <p className="text-xs font-medium uppercase tracking-[0.18em] text-slate-400 dark:text-slate-500">
            Jadon's
          </p>
          <p className="text-lg font-semibold text-tangerine-600 dark:text-tangerine-400">
            Excuses
          </p>
        </div>
        <nav className="flex flex-1 flex-col gap-1">
          {NAV.map(({ to, label, icon: Icon }) => (
            <NavLink
              key={to}
              to={to}
              className={({ isActive }) =>
                cn(
                  "group flex items-center gap-3 rounded-xl px-3 py-2 text-sm font-medium transition-colors",
                  isActive
                    ? "bg-tangerine-50 text-tangerine-700 dark:bg-tangerine-950/40 dark:text-tangerine-300"
                    : "text-slate-600 hover:bg-slate-100 hover:text-slate-900 dark:text-slate-400 dark:hover:bg-slate-800 dark:hover:text-slate-100",
                )
              }
            >
              <Icon className="h-4 w-4" />
              {label}
            </NavLink>
          ))}
        </nav>
        <div className="px-2 pt-4 text-xs text-slate-400 dark:text-slate-500">
          v0.1.0 · local · private
        </div>
      </aside>
      <main className="flex-1 overflow-y-auto">
        <Outlet />
      </main>
    </div>
  );
}
