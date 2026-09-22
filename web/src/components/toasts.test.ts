import { afterEach, describe, expect, it, vi } from "vitest";

import { ApiError } from "../api/client.ts";
import { notifyError, useToasts } from "./toasts.ts";

describe("error toasts", () => {
  afterEach(() => {
    vi.useRealTimers();
    useToasts.setState({ toasts: [] });
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
