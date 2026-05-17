import { create } from "zustand";

interface TrackerState {
  isTracking: boolean;
  lastEventAt: number | null;
  setTracking: (value: boolean) => void;
  markEvent: () => void;
}

export const useTrackerStore = create<TrackerState>((set) => ({
  isTracking: false,
  lastEventAt: null,
  setTracking: (value) => set({ isTracking: value }),
  markEvent: () => set({ lastEventAt: Date.now() }),
}));
