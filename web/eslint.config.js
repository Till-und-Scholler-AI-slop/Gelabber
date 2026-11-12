import js from "@eslint/js";
import reactHooks from "eslint-plugin-react-hooks";
import reactRefresh from "eslint-plugin-react-refresh";
import tseslint from "typescript-eslint";
import noLiteralUiText from "./eslint-rules/no-literal-ui-text.js";

// typescript-eslint 8.x has no TS 7 API. `npm run lint` loads
// eslint-ts6-register.cjs so this package sees @typescript/typescript6.

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
);
