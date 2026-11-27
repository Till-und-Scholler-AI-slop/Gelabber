import { playwright } from "@vitest/browser-playwright";
import { configDefaults, defineConfig } from "vitest/config";

// `*.browser.test.tsx` mounts components in Chromium and drives them with
// real focus, keys, clicks and layout; everything else runs in node.
const BROWSER_TESTS = "src/**/*.browser.test.tsx";

export default defineConfig({
  test: {
    projects: [
      {
        extends: true,
        test: {
          name: "node",
          environment: "node",
          exclude: [...configDefaults.exclude, BROWSER_TESTS],
        },
      },
      {
        extends: true,
        test: {
          name: "browser",
          include: [BROWSER_TESTS],
          // After the node tests, so Chromium takes no CPU from their timers.
          sequence: { groupOrder: 1 },
          browser: {
            enabled: true,
            headless: true,
            provider: playwright(),
            // Desktop width: under 640px menus become bottom sheets.
            viewport: { width: 1280, height: 800 },
            instances: [{ browser: "chromium" }],
            screenshotFailures: false,
          },
        },
      },
    ],
  },
});
