import { api } from "../api/client.ts";
import type { DirectMessage } from "./types.ts";

export const listDms = (signal?: AbortSignal) =>
  api<DirectMessage[]>("/dms", { signal });

export const getDm = (id: string, signal?: AbortSignal) =>
  api<DirectMessage>(`/dms/${id}`, { signal });

export const openDm = (userId: string) =>
  api<DirectMessage>("/dms", { method: "POST", body: { user_id: userId } });

/** Message requests to the signed-in user (#563). */
export const listRequests = (signal?: AbortSignal) =>
  api<DirectMessage[]>("/dms/requests", { signal });

export const acceptRequest = (id: string) =>
  api<DirectMessage>(`/dms/${id}/accept`, { method: "POST" });

export const declineRequest = (id: string) =>
  api<void>(`/dms/${id}/decline`, { method: "POST" });
