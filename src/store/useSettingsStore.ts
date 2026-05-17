import { create } from "zustand";

interface SettingsState {
  startAtLogin: boolean;
  trackBrowser: boolean;
  setStartAtLogin: (value: boolean) => void;
  setTrackBrowser: (value: boolean) => void;
}

export const useSettingsStore = create<SettingsState>((set) => ({
  startAtLogin: false,
  trackBrowser: true,
  setStartAtLogin: (value) => set({ startAtLogin: value }),
  setTrackBrowser: (value) => set({ trackBrowser: value }),
}));
