import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const api = vi.hoisted(() => vi.fn());
vi.mock("../api/client.ts", async (original) => ({
  ...(await original<typeof import("../api/client.ts")>()),
  api,
}));

import { useSession } from "../auth/store.ts";
import type { User } from "../auth/types.ts";
import { changeAccountLocale, followAccountLocale } from "./account.ts";
import { currentLocale, setLocale } from "./locale.ts";

const lena: User = {
  id: "u1",
  email: "lena@example.com",
  name: "Lena",
  avatar_url: null,
  created_at: "2026-01-01T00:00:00Z",
  locale: "de",
};

beforeEach(() => {
  setLocale("de", { persist: false });
  useSession.setState({ status: "authenticated", user: lena });
});
afterEach(() => {
  api.mockReset();
  useSession.setState({ status: "unknown", user: null });
  setLocale("de", { persist: false });
});

describe("changeAccountLocale", () => {
  it("switches at once and keeps the saved language on the user", async () => {
    let answer!: (value: unknown) => void;
    api.mockReturnValue(new Promise((resolve) => (answer = resolve)));
    const saving = changeAccountLocale("en");
    expect(currentLocale()).toBe("en");
    expect(api).toHaveBeenCalledWith("/me/locale", {
      method: "PUT",
      body: { locale: "en" },
    });
    answer({ locale: "en" });
    expect(await saving).toBe(true);
    expect(currentLocale()).toBe("en");
    expect(useSession.getState().user?.locale).toBe("en");
  });

  it("switches back when the save fails", async () => {
    api.mockRejectedValue(new Error("offline"));
    const saving = changeAccountLocale("en");
    expect(currentLocale()).toBe("en");
    expect(await saving).toBe(false);
    expect(currentLocale()).toBe("de");
    expect(useSession.getState().user?.locale).toBe("de");
  });
});

describe("overlapping changeAccountLocale", () => {
  function answers() {
    const calls: Array<{
      resolve: (value: unknown) => void;
      reject: (error: unknown) => void;
    }> = [];
    api.mockImplementation(
      () => new Promise((resolve, reject) => calls.push({ resolve, reject })),
    );
    return calls;
  }

  it("keeps the newer choice when the older answer arrives last", async () => {
    const calls = answers();
    const stop = followAccountLocale();
    try {
      const first = changeAccountLocale("en");
      const second = changeAccountLocale("de");
      calls[1].resolve({ locale: "de" });
      expect(await second).toBe(true);
      calls[0].resolve({ locale: "en" });
      expect(await first).toBe(true);
      expect(currentLocale()).toBe("de");
      expect(useSession.getState().user?.locale).toBe("de");
    } finally {
      stop();
    }
  });

  it("switches back to the saved language when both saves fail", async () => {
    const calls = answers();
    const stop = followAccountLocale();
    try {
      const first = changeAccountLocale("en");
      const second = changeAccountLocale("de");
      calls[0].reject(new Error("offline"));
      expect(await first).toBe(false);
      expect(currentLocale()).toBe("de");
      calls[1].reject(new Error("offline"));
      expect(await second).toBe(false);
      expect(currentLocale()).toBe("de");
      expect(useSession.getState().user?.locale).toBe("de");
    } finally {
      stop();
    }
  });

  it("falls back to an older saved choice when the newer save fails", async () => {
    const calls = answers();
    const stop = followAccountLocale();
    try {
      const first = changeAccountLocale("en");
      const second = changeAccountLocale("de");
      calls[0].resolve({ locale: "en" });
      expect(await first).toBe(true);
      expect(currentLocale()).toBe("de");
      calls[1].reject(new Error("offline"));
      expect(await second).toBe(false);
      expect(currentLocale()).toBe("en");
      expect(useSession.getState().user?.locale).toBe("en");
    } finally {
      stop();
    }
  });
});

describe("followAccountLocale", () => {
  it("shows the language of the account that signs in, and keeps it after", () => {
    useSession.setState({ status: "anonymous", user: null });
    const stop = followAccountLocale();
    try {
      useSession.setState({
        status: "authenticated",
        user: { ...lena, locale: "en" },
      });
      expect(currentLocale()).toBe("en");
      useSession.setState({ status: "anonymous", user: null });
      expect(currentLocale()).toBe("en");
    } finally {
      stop();
    }
  });

  it("leaves a choice in flight alone while the account stays the same", () => {
    const stop = followAccountLocale();
    try {
      setLocale("en", { persist: false });
      // A profile save answers with the user as the server still has it.
      useSession.setState({ user: { ...lena, name: "Lena B." } });
      expect(currentLocale()).toBe("en");
    } finally {
      stop();
    }
  });
});
