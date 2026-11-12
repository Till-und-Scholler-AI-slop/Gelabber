import { describe, expect, it } from "vitest";

import { APP_VERSION } from "../version.ts";
import {
  PAGE_MAX,
  feedbackContext,
  feedbackPage,
  osFromUserAgent,
} from "./context.ts";

describe("osFromUserAgent", () => {
  // The User-Agents of the API's own tests (auth/device.rs).
  it.each([
    [
      "Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:132.0) Gecko/20100101 Firefox/132.0",
      "windows",
    ],
    [
      "Mozilla/5.0 (X11; Linux x86_64; rv:132.0) Gecko/20100101 Firefox/132.0",
      "linux",
    ],
    [
      "Mozilla/5.0 (Macintosh; Intel Mac OS X 14.7; rv:132.0) Gecko/20100101 Firefox/132.0",
      "macos",
    ],
    [
      "Mozilla/5.0 (Android 14; Mobile; rv:132.0) Gecko/132.0 Firefox/132.0",
      "android",
    ],
    [
      "Mozilla/5.0 (Linux; Android 10; K) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Mobile Safari/537.36",
      "android",
    ],
    [
      "Mozilla/5.0 (iPhone; CPU iPhone OS 18_1 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.1 Mobile/15E148 Safari/604.1",
      "ios",
    ],
    [
      "Mozilla/5.0 (iPad; CPU OS 18_1 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.1 Mobile/15E148 Safari/604.1",
      "ios",
    ],
    [
      "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/538.1 (KHTML, like Gecko)",
      "linux",
    ],
    ["curl/8.10.1", "other"],
    ["", "other"],
  ])("%s is %s", (agent, os) => {
    expect(osFromUserAgent(agent)).toBe(os);
  });
});

describe("feedbackPage", () => {
  it("drops the query and the fragment", () => {
    expect(feedbackPage("/s/a/c/b?x=1#y")).toBe("/s/a/c/b");
    expect(feedbackPage("/settings/darstellung#top")).toBe(
      "/settings/darstellung",
    );
  });

  it("caps the path at 300 characters", () => {
    const long = `/${"a".repeat(400)}`;
    expect(feedbackPage(long)).toHaveLength(PAGE_MAX);
    expect(feedbackPage(long).startsWith("/aaa")).toBe(true);
  });

  it("sends the root for a path the API would refuse", () => {
    expect(feedbackPage("//evil.example")).toBe("/");
    expect(feedbackPage("")).toBe("/");
  });
});

describe("feedbackContext", () => {
  it("is the web app's five values and nothing else", () => {
    expect(
      feedbackContext(
        "/s/home/c/sofa?q=1",
        "en",
        "Mozilla/5.0 (X11; Linux x86_64; rv:132.0) Gecko/20100101 Firefox/132.0",
      ),
    ).toEqual({
      client: "web",
      os: "linux",
      app_version: APP_VERSION,
      locale: "en",
      page: "/s/home/c/sofa",
    });
  });
});
