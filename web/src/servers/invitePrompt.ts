// "Lade deine Leute ein" right after creating a server. The prompt lives
// outside the page that asked for it: creating navigates away from there,
// and on a phone the asking rail sits in a drawer that closes.

import { create } from "zustand";

export type InvitePrompt = { userId: string; id: string; name: string };

export const useInvitePrompt = create<{ prompt: InvitePrompt | null }>(() => ({
  prompt: null,
}));

export function promptInvite(prompt: InvitePrompt): void {
  useInvitePrompt.setState({ prompt });
}

export function dismissInvitePrompt(): void {
  useInvitePrompt.setState({ prompt: null });
}
