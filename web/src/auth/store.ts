// Who is signed in: the store behind session.ts, which re-exports it. It is
// a leaf so scope.ts and voice/session.ts can read it without importing the
// session API, whose identity changes tear down voice and drafts (#291).

import { create } from "zustand";

import type { User } from "./types.ts";

export type SessionStatus = "unknown" | "anonymous" | "authenticated";

export type SessionState = {
  status: SessionStatus;
  user: User | null;
};

export const useSession = create<SessionState>(() => ({
  status: "unknown",
  user: null,
}));
