// Whether the quick switcher (Strg/⌘+K) is open: the shortcut and the
// overview's "Springen zu …" open the same one.
import { create } from "zustand";

export const useSwitcherOpen = create<{ open: boolean }>(() => ({
  open: false,
}));

export const openSwitcher = () => useSwitcherOpen.setState({ open: true });
export const closeSwitcher = () => useSwitcherOpen.setState({ open: false });
