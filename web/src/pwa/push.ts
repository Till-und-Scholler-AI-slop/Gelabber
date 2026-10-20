// Web Push (#243): with notifications allowed, the browser subscribes with
// its vendor's push service and the server sends direct messages and
// mentions there while no page of the app is in front. public/sw.js shows
// them. Where no service worker runs (vite dev, the desktop app, plain HTTP)
// or the server has no key, nothing changes.
import { useEffect } from "react";

import { api } from "../api/client.ts";
import { activeServiceWorker } from "./register.ts";

type PushRegistration = Pick<ServiceWorkerRegistration, "pushManager">;

export type PushResult = "on" | "off" | "unavailable";

export function decodeKey(key: string): Uint8Array<ArrayBuffer> {
  const base64 = key.replace(/-/g, "+").replace(/_/g, "/");
  const raw = atob(base64 + "=".repeat((4 - (base64.length % 4)) % 4));
  const bytes = new Uint8Array(new ArrayBuffer(raw.length));
  for (let i = 0; i < raw.length; i += 1) bytes[i] = raw.charCodeAt(i);
  return bytes;
}

function sameKey(current: ArrayBuffer | null | undefined, key: Uint8Array) {
  if (!current) return false;
  const bytes = new Uint8Array(current);
  return bytes.length === key.length && bytes.every((b, i) => b === key[i]);
}

/**
 * Subscribe this browser and tell the server, for the signed-in session.
 * Again on every start: a subscription follows the session that sent it, so
 * the server drops it on sign-out and another account can take it over.
 */
export async function syncPush(
  registration?: PushRegistration,
): Promise<PushResult> {
  if (
    typeof Notification === "undefined" ||
    Notification.permission !== "granted"
  )
    return "off";
  const worker = registration ?? (await activeServiceWorker());
  if (!worker?.pushManager) return "unavailable";
  const { key } = await api<{ key: string | null }>("/push/key");
  if (!key) return "unavailable";
  const serverKey = decodeKey(key);
  let subscription = await worker.pushManager.getSubscription();
  if (
    subscription &&
    !sameKey(subscription.options.applicationServerKey, serverKey)
  ) {
    // The server got a new key; the old subscription is of no use.
    await subscription.unsubscribe().catch(() => false);
    subscription = null;
  }
  subscription ??= await worker.pushManager.subscribe({
    userVisibleOnly: true,
    applicationServerKey: serverKey,
  });
  const { endpoint, keys } = subscription.toJSON();
  await api("/push/subscriptions", {
    method: "POST",
    body: { endpoint, keys },
  });
  return "on";
}

/** Subscribe once per account, and as soon as notifications get allowed. */
export function usePushSubscription(userId: string | null): void {
  useEffect(() => {
    if (!userId || typeof Notification === "undefined") return undefined;
    let done = false;
    const check = () => {
      if (done || Notification.permission !== "granted") return;
      done = true;
      void syncPush().catch(() => {
        // Push is extra. Try again on the next start.
      });
    };
    check();
    window.addEventListener("focus", check);
    document.addEventListener("visibilitychange", check);
    return () => {
      window.removeEventListener("focus", check);
      document.removeEventListener("visibilitychange", check);
    };
  }, [userId]);
}
