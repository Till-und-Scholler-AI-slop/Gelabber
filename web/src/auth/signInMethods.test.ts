import { afterEach, describe, expect, it } from "vitest";

import { setLocale } from "../i18n/locale.ts";
import {
  disconnectConfirm,
  oidcErrorRows,
  signInFeedback,
  signInProvider,
} from "./signInMethods.ts";
import type { SignInMethods } from "./types.ts";

afterEach(() => setLocale("de", { persist: false }));

const google = (connected: boolean, configured = true) => ({
  id: "google",
  name: "Google",
  configured,
  connected: connected
    ? { email: "lena@example.test", since: "2026-11-10T10:00:00Z" }
    : null,
});

describe("what a link flow reports back to Sicherheit", () => {
  it("keeps a provider id and a known oidc_error", () => {
    expect(
      signInFeedback({ connected: "google", oidc_error: "identity_taken" }),
    ).toEqual({ connected: "google", oidcError: "identity_taken" });
  });

  it("drops anything else", () => {
    expect(signInFeedback({ oidc_error: "registration_closed" })).toEqual({});
    expect(signInFeedback({ connected: "<b>x</b>" })).toEqual({});
    expect(signInFeedback({ connected: 3 })).toEqual({});
    expect(signInFeedback(undefined)).toEqual({});
    expect(signInFeedback(null)).toEqual({});
  });
});

describe("the provider a password-less account signs in with", () => {
  it("is a connected one, preferring one still configured", () => {
    const methods: SignInMethods = {
      password: false,
      providers: [
        { ...google(true, false), id: "old", name: "Alt" },
        google(true),
      ],
    };
    expect(signInProvider(methods)).toEqual({ id: "google", name: "Google" });
  });

  it("falls back to one the operator turned off", () => {
    expect(
      signInProvider({ password: false, providers: [google(true, false)] }),
    ).toEqual({ id: "google", name: "Google" });
  });

  it("is none while nothing is connected", () => {
    expect(
      signInProvider({ password: true, providers: [google(false)] }),
    ).toBeNull();
  });
});

describe("the rows an oidc_error shows on", () => {
  it("is the rows that could have started a link", () => {
    expect(
      oidcErrorRows({
        password: true,
        providers: [{ ...google(true), id: "a" }, google(false)],
      }),
    ).toEqual(["google"]);
  });

  it("is the first row when every provider is connected", () => {
    expect(
      oidcErrorRows({ password: true, providers: [google(true)] }),
    ).toEqual(["google"]);
    expect(oidcErrorRows({ password: true, providers: [] })).toEqual([]);
  });
});

describe("the question before Trennen", () => {
  it("names the provider, in the interface's language", () => {
    expect(disconnectConfirm("Google")).toEqual({
      title: "Google trennen?",
      message:
        "Du meldest dich dann nicht mehr mit Google an. Verbinden kannst du es jederzeit wieder.",
      confirmLabel: "Trennen",
    });
    setLocale("en", { persist: false });
    expect(disconnectConfirm("Google").title).toBe("Disconnect Google?");
  });
});
