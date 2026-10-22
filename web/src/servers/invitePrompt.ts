// "Lade deine Leute ein" right after creating a server. The prompt lives
// outside the dialog that asked for it: creating closes that dialog and
// navigates away from the page it was opened on.

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
