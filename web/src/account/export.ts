// The data export of an account (#519): `GET`/`POST /api/me/export` and
// the download, and how the row in Sicherheit → Kontodaten words its state.

import { queryOptions, useQuery } from "@tanstack/react-query";

import { API_BASE, ApiError, api } from "../api/client.ts";
import { errorMessage } from "../auth/rules.ts";
import { translate } from "../i18n/catalog.ts";
import { account } from "../i18n/catalogs/account.ts";
import { formatDate, formatRelative } from "../i18n/format.ts";
import { formatSize } from "../messages/rules.ts";

export type ExportState = "pending" | "running" | "ready" | "failed";

/** The account's one export, as the API shows it. */
export type ExportView = {
  state: ExportState;
  requested_at: string;
  finished_at: string | null;
  expires_at: string | null;
  size_bytes: number | null;
};

/** The export, if any, and whether a mail announces it when ready. */
export type ExportStatus = { export: ExportView | null; mail: boolean };

/** A plain link: the archive comes as a download (`attachment`). */
export const EXPORT_DOWNLOAD_URL = `${API_BASE}/me/export/download`;

/** While an export is being built, the page asks again this often. */
export const EXPORT_POLL_MS = 10_000;

export function getExport(signal?: AbortSignal): Promise<ExportStatus> {
  return api<ExportStatus>("/me/export", signal ? { signal } : {});
}

/** Asks for a new archive (202), replacing a failed or day-old one. */
export function requestExport(): Promise<ExportStatus> {
  return api<ExportStatus>("/me/export", { method: "POST" });
}

export function exportKey(userId: string) {
  return ["account", "export", userId] as const;
}

export function isBuilding(status: ExportStatus | undefined): boolean {
  const state = status?.export?.state;
  return state === "pending" || state === "running";
}

/**
 * The export of "Kontodaten". While it is being built it is read again
 * every 10 s, and only while the page is visible (the query's interval
 * pauses in a background tab).
 */
export function useExport(userId: string) {
  return useQuery(exportQueryOptions(userId));
}

export function exportQueryOptions(userId: string) {
  return queryOptions({
    queryKey: exportKey(userId),
    queryFn: ({ signal }) => getExport(signal),
    refetchInterval: (query) =>
      isBuilding(query.state.data) ? EXPORT_POLL_MS : false,
  });
}

/** What the row's button does in each state. */
export type ExportAction = "request" | "building" | "download" | "retry";

/** The row's hint and button for an export state. */
export function exportRow(status: ExportStatus): {
  hint: string;
  action: ExportAction;
} {
  const view = status.export;
  const mail = translate(account, "export.hint.mail");
  switch (view?.state) {
    case undefined:
      return {
        hint: `${translate(account, "export.hint.none")} ${
          status.mail ? mail : translate(account, "export.hint.noMail")
        }`,
        action: "request",
      };
    case "pending":
    case "running": {
      const building = translate(account, "export.hint.building");
      return {
        hint: status.mail ? `${building} ${mail}` : building,
        action: "building",
      };
    }
    case "ready":
      return {
        hint: translate(account, "export.hint.ready", {
          date: view.expires_at ? formatDate(view.expires_at) : "",
          size: formatSize(view.size_bytes ?? 0),
        }),
        action: "download",
      };
    case "failed":
      return {
        hint: translate(account, "export.hint.failed"),
        action: "retry",
      };
  }
}

/**
 * Why a request was refused: a younger ready export (429) says when the
 * next one can be asked for; anything else is the standard error line.
 */
export function exportRequestError(error: unknown): string | null {
  if (error instanceof DOMException && error.name === "AbortError") return null;
  if (!(error instanceof ApiError)) return errorMessage("internal");
  if (error.code === "rate_limited" && error.retryAfter)
    return translate(account, "export.tooSoon", {
      when: waitWords(error.retryAfter),
    });
  return errorMessage(error.code);
}

/** "in 5 Stunden", "in 12 Minuten", "in 30 Sekunden". */
function waitWords(seconds: number): string {
  if (seconds < 60) return formatRelative(Math.ceil(seconds), "second");
  if (seconds < 3600) return formatRelative(Math.ceil(seconds / 60), "minute");
  return formatRelative(Math.ceil(seconds / 3600), "hour");
}
