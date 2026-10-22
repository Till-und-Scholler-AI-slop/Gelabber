import { afterEach, describe, expect, it, vi } from "vitest";

import { ApiError } from "../api/client.ts";
import {
  ERROR_MS,
  INFO_MS,
  clearToasts,
  notify,
  notifyError,
  useToasts,
} from "./toasts.ts";

describe("error toasts", () => {
  afterEach(() => {
    clearToasts();
    vi.useRealTimers();
  });

  it("shows one toast for a burst of the same failure", () => {
    for (let i = 0; i < 20; i++) {
      notifyError(new Error("Der Stream konnte nicht verbunden werden."));
    }
    expect(useToasts.getState().toasts).toHaveLength(1);
    expect(useToasts.getState().toasts[0]?.message).toBe(
      "Der Stream konnte nicht verbunden werden.",
    );
  });

  it("shows the field message when an upload is rejected", () => {
    notifyError(
      new ApiError("validation_failed", 422, "Validation failed.", {
        size: "invalid",
      }),
    );
    expect(useToasts.getState().toasts[0]?.message).toBe("Datei ist ungültig.");
  });

  it("names the change that failed before the reason", () => {
    notifyError(
      new ApiError("network", 0, "Network error."),
      "Nachricht nicht gelöscht.",
    );
    // A different change failing for the same reason is its own notice.
    notifyError(
      new ApiError("network", 0, "Network error."),
      "Änderung nicht gespeichert.",
    );
    expect(useToasts.getState().toasts.map((toast) => toast.message)).toEqual([
      "Nachricht nicht gelöscht. Keine Verbindung zum Server.",
      "Änderung nicht gespeichert. Keine Verbindung zum Server.",
    ]);
  });

  it("keeps the ApiError text and caps a stack of different errors", () => {
    notifyError(
      new ApiError("forbidden", 403, "Dafür fehlt dir die Berechtigung."),
    );
    notifyError(new Error("erster"));
    notifyError(new Error("zweiter"));
    notifyError(new Error("dritter"));
    notifyError(new Error("vierter"));
    const messages = useToasts.getState().toasts.map((toast) => toast.message);
    expect(messages).toEqual([
      "Dafür fehlt dir die Berechtigung.",
      "erster",
      "zweiter",
    ]);
  });
});

describe("toast timing", () => {
  afterEach(() => {
    clearToasts();
    vi.useRealTimers();
  });

  const shown = () => useToasts.getState().toasts.map((toast) => toast.message);

  it("lets a notice go after four seconds and an error after ten", () => {
    vi.useFakeTimers();
    notify("Kopiert.");
    notifyError(new Error("Fehlgeschlagen."));
    vi.advanceTimersByTime(INFO_MS);
    expect(shown()).toEqual(["Fehlgeschlagen."]);
    vi.advanceTimersByTime(ERROR_MS - INFO_MS - 1);
    expect(shown()).toEqual(["Fehlgeschlagen."]);
    vi.advanceTimersByTime(1);
    expect(shown()).toEqual([]);
  });

  it("never shortens an error below ten seconds", () => {
    vi.useFakeTimers();
    useToasts.getState().push("error", "Kurz?", { duration: 2000 });
    vi.advanceTimersByTime(ERROR_MS - 1);
    expect(shown()).toEqual(["Kurz?"]);
  });

  it("holds a toast while hovered or focused, and resumes after both end", () => {
    vi.useFakeTimers();
    const id = useToasts.getState().push("info", "Lies mich.")!;
    vi.advanceTimersByTime(3000);
    useToasts.getState().hold(id, "hover");
    useToasts.getState().hold(id, "focus");
    vi.advanceTimersByTime(60_000);
    expect(shown()).toEqual(["Lies mich."]);
    useToasts.getState().release(id, "hover");
    vi.advanceTimersByTime(60_000);
    expect(shown()).toEqual(["Lies mich."]);
    useToasts.getState().release(id, "focus");
    // The second left before the hold, but never less than a moment.
    vi.advanceTimersByTime(1499);
    expect(shown()).toEqual(["Lies mich."]);
    vi.advanceTimersByTime(1);
    expect(shown()).toEqual([]);
  });

  it("keeps the remaining time across a short hold", () => {
    vi.useFakeTimers();
    const id = useToasts.getState().push("info", "Weiter.")!;
    vi.advanceTimersByTime(1000);
    useToasts.getState().hold(id, "hover");
    vi.advanceTimersByTime(5000);
    useToasts.getState().release(id, "hover");
    vi.advanceTimersByTime(INFO_MS - 1000 - 1);
    expect(shown()).toEqual(["Weiter."]);
    vi.advanceTimersByTime(1);
    expect(shown()).toEqual([]);
  });

  it("keeps every undo offer, even with the same text", () => {
    const undone: string[] = [];
    for (const id of ["a", "b"])
      notify("Link widerrufen", {
        action: { label: "Rückgängig", onClick: () => undone.push(id) },
      });
    notify("Kopiert.");
    notify("Kopiert.");
    const toasts = useToasts.getState().toasts;
    expect(toasts.map((toast) => toast.message)).toEqual([
      "Link widerrufen",
      "Link widerrufen",
      "Kopiert.",
    ]);
    for (const toast of toasts) toast.action?.onClick();
    expect(undone).toEqual(["a", "b"]);
  });

  it("carries an action for undo", () => {
    let undone = false;
    notify("Link widerrufen", {
      action: { label: "Rückgängig", onClick: () => (undone = true) },
    });
    useToasts.getState().toasts[0]?.action?.onClick();
    expect(undone).toBe(true);
  });
});
