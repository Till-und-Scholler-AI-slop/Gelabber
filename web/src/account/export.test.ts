import { afterEach, describe, expect, it } from "vitest";

import { ApiError } from "../api/client.ts";
import { setLocale } from "../i18n/locale.ts";
import {
  EXPORT_POLL_MS,
  exportKey,
  exportQueryOptions,
  exportRequestError,
  exportRow,
  isBuilding,
  type ExportStatus,
  type ExportView,
} from "./export.ts";

const view = (state: ExportView["state"], extra: Partial<ExportView> = {}) =>
  ({
    state,
    requested_at: "2026-11-10T18:00:00Z",
    finished_at: null,
    expires_at: null,
    size_bytes: null,
    ...extra,
  }) satisfies ExportView;

const READY = view("ready", {
  finished_at: "2026-11-10T18:05:00Z",
  expires_at: "2026-11-17T12:00:00Z",
  size_bytes: 2_621_440,
});

afterEach(() => setLocale("de", { persist: false }));

describe("the export row", () => {
  it("offers a first export, announced by mail or on this page", () => {
    expect(exportRow({ export: null, mail: true })).toEqual({
      hint: "Deine Nachrichten, Dateien und Kontodaten als Archiv. Du bekommst eine E-Mail, sobald es bereitsteht.",
      action: "request",
    });
    expect(exportRow({ export: null, mail: false })).toEqual({
      hint: "Deine Nachrichten, Dateien und Kontodaten als Archiv. Du kannst es hier herunterladen, sobald es bereitsteht.",
      action: "request",
    });
  });

  it("waits while the archive is being built", () => {
    for (const state of ["pending", "running"] as const) {
      expect(exportRow({ export: view(state), mail: true })).toEqual({
        hint: "Wird erstellt. Das kann eine Weile dauern. Du bekommst eine E-Mail, sobald es bereitsteht.",
        action: "building",
      });
      expect(exportRow({ export: view(state), mail: false })).toEqual({
        hint: "Wird erstellt. Das kann eine Weile dauern.",
        action: "building",
      });
    }
  });

  it("offers the download until it expires, with its size", () => {
    expect(exportRow({ export: READY, mail: true })).toEqual({
      hint: "Bereit bis 17. November 2026, 2,5 MB.",
      action: "download",
    });
    setLocale("en", { persist: false });
    expect(exportRow({ export: READY, mail: true }).hint).toBe(
      "Ready until 17 November 2026, 2.5 MB.",
    );
  });

  it("offers to ask again after a failure", () => {
    expect(exportRow({ export: view("failed"), mail: true })).toEqual({
      hint: "Der Export ist fehlgeschlagen.",
      action: "retry",
    });
  });

  it("says when a refused request can be made again", () => {
    const tooSoon = (seconds: number) =>
      exportRequestError(
        new ApiError("rate_limited", 429, "Too many.", {}, seconds),
      );
    expect(tooSoon(5 * 3600 - 30)).toBe(
      "Einen neuen Export kannst du in 5 Stunden anfordern.",
    );
    expect(tooSoon(90)).toBe(
      "Einen neuen Export kannst du in 2 Minuten anfordern.",
    );
    expect(
      exportRequestError(new ApiError("export_pending", 409, "Pending.")),
    ).toBe("Dein Export wird schon erstellt.");
    expect(
      exportRequestError(new DOMException("aborted", "AbortError")),
    ).toBeNull();
    setLocale("en", { persist: false });
    expect(tooSoon(3600)).toBe("You can request a new export in 1 hour.");
  });
});

describe("polling the export", () => {
  it("is building only while pending or running", () => {
    expect(isBuilding(undefined)).toBe(false);
    expect(isBuilding({ export: null, mail: true })).toBe(false);
    expect(isBuilding({ export: view("pending"), mail: true })).toBe(true);
    expect(isBuilding({ export: view("running"), mail: true })).toBe(true);
    expect(isBuilding({ export: READY, mail: true })).toBe(false);
    expect(isBuilding({ export: view("failed"), mail: true })).toBe(false);
  });

  it("asks every 10 s while it is built, only while the page is visible", () => {
    const options = exportQueryOptions("u1");
    expect(options.queryKey).toEqual(exportKey("u1"));
    // TanStack pauses an interval in a hidden tab unless this is set.
    expect(options).not.toHaveProperty("refetchIntervalInBackground");
    const interval = options.refetchInterval as (query: {
      state: { data: ExportStatus | undefined };
    }) => number | false;
    const every = (data: ExportStatus | undefined) =>
      interval({ state: { data } });
    expect(every(undefined)).toBe(false);
    expect(every({ export: null, mail: true })).toBe(false);
    expect(every({ export: view("pending"), mail: true })).toBe(EXPORT_POLL_MS);
    expect(every({ export: view("running"), mail: false })).toBe(
      EXPORT_POLL_MS,
    );
    expect(every({ export: READY, mail: true })).toBe(false);
    expect(every({ export: view("failed"), mail: true })).toBe(false);
    expect(EXPORT_POLL_MS).toBe(10_000);
  });
});
