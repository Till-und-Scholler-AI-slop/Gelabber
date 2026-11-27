// The voice modules' module-state check that `npm run lint` runs
// (eslint.config.js).
import { RuleTester } from "eslint";
import { describe, it } from "vitest";

import rule from "./no-module-state.js";

RuleTester.describe = describe;
RuleTester.it = it;
RuleTester.itOnly = it.only;

const tester = new RuleTester({
  languageOptions: { ecmaVersion: "latest", sourceType: "module" },
});

tester.run("no-module-state", rule, {
  valid: [
    // Constants and functions are no state.
    "const LIMIT = 3;",
    "export const TEXTS = { a: 1 };",
    "const make = () => new Map();",
    "export function reset() { let count = 0; return count; }",
    // A call that may return a new object is the module's business.
    "const storage = createStorage();",
    // Inside a function, a block or a class it lives as long as that.
    "function f() { const seen = new Set(); return seen; }",
    "{ let local = 1; }",
    "class Seat { peers = new Map(); }",
    // The session itself.
    { code: "export let voice = create();", options: [{ allow: ["voice"] }] },
  ],
  invalid: [
    {
      code: "let pending = null;",
      errors: [{ messageId: "variable", data: { name: "pending" } }],
    },
    {
      code: "export let current;",
      errors: [{ messageId: "variable", data: { name: "current" } }],
    },
    {
      code: "var count = 0;",
      errors: [{ messageId: "variable", data: { name: "count" } }],
    },
    {
      code: "let { a, b } = defaults;",
      errors: [{ messageId: "variable", data: { name: "{ a, b }" } }],
    },
    {
      code: "const peers = new Map();",
      errors: [{ messageId: "instance", data: { name: "peers" } }],
    },
    {
      code: "export const retry = new MediaRetry(), LIMIT = 3;",
      errors: [{ messageId: "instance", data: { name: "retry" } }],
    },
    {
      // Without `allow`, the session's own name is module state too.
      code: "export let voice = create();",
      errors: [{ messageId: "variable", data: { name: "voice" } }],
    },
    {
      // `allow` names only its own variables.
      code: "export let voice = create(); let other = 1;",
      options: [{ allow: ["voice"] }],
      errors: [{ messageId: "variable", data: { name: "other" } }],
    },
  ],
});
