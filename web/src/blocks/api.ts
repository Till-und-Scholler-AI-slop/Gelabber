import { api } from "../api/client.ts";
import type { BlockedUser } from "./types.ts";

export const listBlocks = (signal?: AbortSignal) =>
  api<BlockedUser[]>("/users/me/blocks", { signal });

export const block = (userId: string) =>
  api<void>(`/users/me/blocks/${userId}`, { method: "PUT" });

export const unblock = (userId: string) =>
  api<void>(`/users/me/blocks/${userId}`, { method: "DELETE" });
