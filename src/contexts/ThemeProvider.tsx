import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useState,
  type ReactNode,
} from "react";
import { invoke } from "@tauri-apps/api/core";

export type ThemePreference = "light" | "dark" | "system";

type ThemeContextValue = {
  /** Stored preference (what the user chose). */
  theme: ThemePreference;
  /** Persist preference and sync document class + SQLite. */
  setTheme: (t: ThemePreference) => Promise<void>;
  /** Resolved light/dark for charts and conditional styling. */
  resolvedTheme: "light" | "dark";
};

const ThemeContext = createContext<ThemeContextValue | null>(null);

export function chartPalette(resolved: "light" | "dark") {
  return resolved === "dark"
    ? {
        grid: "#475569",
        tick: "#94a3b8",
        tooltipBg: "#1e293b",
        tooltipBorder: "#334155",
      }
    : {
        grid: "#e5e7eb",
        tick: "#64748b",
        tooltipBg: "#ffffff",
        tooltipBorder: "#e2e8f0",
      };
}

export function ThemeProvider({ children }: { children: ReactNode }) {
  const [theme, setThemeState] = useState<ThemePreference>("system");
  const [systemDark, setSystemDark] = useState(() =>
    typeof window !== "undefined"
      ? window.matchMedia("(prefers-color-scheme: dark)").matches
      : false,
  );

  useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const t = await invoke<string | null>("get_setting", { key: "theme" });
        if (cancelled) return;
        if (t === "light" || t === "dark" || t === "system") {
          setThemeState(t);
        }
      } catch {
        /* keep default */
      }
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  useEffect(() => {
    const mq = window.matchMedia("(prefers-color-scheme: dark)");
    const onChange = () => setSystemDark(mq.matches);
    mq.addEventListener("change", onChange);
    return () => mq.removeEventListener("change", onChange);
  }, []);

  const resolvedTheme = useMemo((): "light" | "dark" => {
    if (theme === "dark") return "dark";
    if (theme === "light") return "light";
    return systemDark ? "dark" : "light";
  }, [theme, systemDark]);

  useEffect(() => {
    document.documentElement.classList.toggle(
      "dark",
      resolvedTheme === "dark",
    );
  }, [resolvedTheme]);

  const setTheme = useCallback(async (t: ThemePreference) => {
    setThemeState(t);
    await invoke("set_setting", { key: "theme", value: t });
  }, []);

  const value = useMemo(
    () => ({ theme, setTheme, resolvedTheme }),
    [theme, setTheme, resolvedTheme],
  );

  return (
    <ThemeContext.Provider value={value}>{children}</ThemeContext.Provider>
  );
}

export function useTheme(): ThemeContextValue {
  const ctx = useContext(ThemeContext);
  if (!ctx) {
    throw new Error("useTheme must be used within ThemeProvider");
  }
  return ctx;
}
