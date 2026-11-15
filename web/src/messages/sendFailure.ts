// Why a send failed, in words the failed row can show (#581). Where the
// server named a reason it is told; otherwise the stage's generic text,
// with the status and code for a refusal so it is not a dead end.

import { ApiError } from "../api/client.ts";
import { translate } from "../i18n/catalog.ts";
import { chat } from "../i18n/catalogs/chat.ts";
import type { SendStage } from "./pending.ts";

const generic = {
  presign: "send.presignFailed",
  upload: "send.uploadFailed",
  bind: "send.bindFailed",
} as const;

export function sendFailureText(
  error: unknown,
  stage: SendStage,
  gif: boolean,
): string {
  const fallback = translate(chat, generic[stage]);
  if (!(error instanceof ApiError)) return fallback;
  if (error.fields.gif === "invalid") return translate(chat, "send.gifGone");
  if (error.fields.gif === "unavailable") return translate(chat, "send.gifOff");
  if (error.code === "rate_limited") {
    const wait = error.retryAfter;
    if (wait !== null && wait > 0) {
      const count = Math.ceil(wait);
      return gif
        ? translate(chat, "send.gifRateLimitedWait", { count })
        : translate(chat, "send.rateLimitedWait", { count });
    }
    return translate(chat, gif ? "send.gifRateLimited" : "send.rateLimited");
  }
  if (error.code === "forbidden") return translate(chat, "send.forbidden");
  if (error.status >= 400 && error.status < 500)
    return `${fallback} (${error.status} ${error.code})`;
  return fallback;
}
