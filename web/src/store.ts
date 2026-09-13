import { create } from "zustand";

type AppState = {
  name: string;
};

export const useAppStore = create<AppState>(() => ({
  name: "Gelabber",
}));
