/* global URL */
import js from "@eslint/js";
import reactHooks from "eslint-plugin-react-hooks";
import reactRefresh from "eslint-plugin-react-refresh";
import tseslint from "typescript-eslint";
import { existsSync, readdirSync, readFileSync } from "node:fs";

import noLiteralUiText from "./eslint-rules/no-literal-ui-text.js";

// typescript-eslint 8.x has no TS 7 API. `npm run lint` loads
// eslint-ts6-register.cjs so this package sees @typescript/typescript6.

// Files whose text is not in the string catalogs yet: one list per
// extraction batch (docs/i18n.md), each deleted by its batch when done.
const pendingDir = new URL("./src/i18n/pending/", import.meta.url);
const pendingI18n = existsSync(pendingDir)
  ? readdirSync(pendingDir)
      .filter((name) => name.endsWith(".txt"))
      .flatMap((name) =>
        readFileSync(new URL(name, pendingDir), "utf8")
          .split("\n")
          .map((line) => line.trim())
          .filter((line) => line && !line.startsWith("#")),
      )
  : [];

export default tseslint.config(
  { ignores: ["dist", "eslint-ts6-register.cjs"] },
  js.configs.recommended,
  ...tseslint.configs.recommended,
  {
    files: ["**/*.{ts,tsx}"],
    plugins: {
      "react-hooks": reactHooks,
      "react-refresh": reactRefresh,
    },
    rules: {
      ...reactHooks.configs.recommended.rules,
      "react-refresh/only-export-components": [
        "warn",
        { allowConstantExport: true },
      ],
    },
  },
  {
    files: ["src/**/*.{ts,tsx}"],
    ignores: ["**/*.test.*", "src/i18n/catalogs/**"],
    plugins: { gelabber: { rules: { "no-literal-ui-text": noLiteralUiText } } },
    rules: { "gelabber/no-literal-ui-text": "error" },
  },
  ...(pendingI18n.length
    ? [{ files: pendingI18n, rules: { "gelabber/no-literal-ui-text": "off" } }]
    : []),
);
