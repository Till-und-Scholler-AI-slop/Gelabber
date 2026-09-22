// Generation of the signed-in identity. Bumped on every user-id change,
// including session end, so a mutation that started as user A cannot commit
// after A is gone — even if A signs back in before the response arrives.

import { useSession } from "./session.ts";

export type ScopeStamp = {
  userId: string;
  generation: number;
};

let generation = 0;

export function scopeGeneration(): number {
  return generation;
}

export function nextScopeGeneration(): number {
  generation += 1;
  return generation;
}

export function resetScopeGenerationForTests(): void {
  generation = 0;
}

/** Who the tab is right now, or nothing when signed out. */
export function currentUserId(): string | null {
  return useSession.getState().user?.id ?? null;
}

/** Capture the user a mutation is allowed to write for. */
export function takeStamp(): ScopeStamp | undefined {
  const userId = currentUserId();
  if (!userId) return undefined;
  return { userId, generation };
}

export function stampHolds<T extends { userId?: string; generation?: number }>(
  stamp: T | null | undefined,
): stamp is T & ScopeStamp {
  return (
    stamp != null &&
    typeof stamp.userId === "string" &&
    stamp.userId.length > 0 &&
    typeof stamp.generation === "number" &&
    stamp.userId === currentUserId() &&
    stamp.generation === generation
  );
}

export function useUserId(): string {
  return useSession((state) => state.user?.id ?? "");
}
