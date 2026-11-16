import { api } from "../api/client.ts";
import type { FriendList, SendAnswer } from "./types.ts";

export const listFriends = (signal?: AbortSignal) =>
  api<FriendList>("/friends", { signal });

export const sendRequest = (target: { user_id: string } | { code: string }) =>
  api<SendAnswer>("/friends/requests", { method: "POST", body: target });

export const acceptRequest = (userId: string) =>
  api<void>(`/friends/requests/${userId}/accept`, { method: "POST" });

export const declineRequest = (userId: string) =>
  api<void>(`/friends/requests/${userId}/decline`, { method: "POST" });

export const withdrawRequest = (userId: string) =>
  api<void>(`/friends/requests/${userId}`, { method: "DELETE" });

export const removeFriend = (userId: string) =>
  api<void>(`/friends/${userId}`, { method: "DELETE" });

export const getFriendCode = (signal?: AbortSignal) =>
  api<{ code: string }>("/me/friend-code", { signal });

export const regenerateFriendCode = () =>
  api<{ code: string }>("/me/friend-code", { method: "POST" });
