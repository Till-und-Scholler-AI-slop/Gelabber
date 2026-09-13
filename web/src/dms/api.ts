import { api } from "../api/client.ts";
import type { DirectMessage } from "./types.ts";

export const listDms = (signal?: AbortSignal) =>
  api<DirectMessage[]>("/dms", { signal });

export const getDm = (id: string, signal?: AbortSignal) =>
  api<DirectMessage>(`/dms/${id}`, { signal });

export const openDm = (userId: string) =>
  api<DirectMessage>("/dms", { method: "POST", body: { user_id: userId } });
