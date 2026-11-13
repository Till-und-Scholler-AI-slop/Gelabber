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
  // Text fields come from components/ui (Input, Textarea, Select, with
  // FieldShell around several parts), so every field has the same one
  // frame and one focus style on the --lr-field-* tokens (#561).
  // Checkboxes, radios, sliders, file and colour pickers stay native.
  // The prerendered public pages ship without ui.css and style their few
  // fields in public.css on the same tokens.
  {
    files: ["src/**/*.tsx"],
    ignores: ["**/*.test.*", "src/components/ui/**", "src/public/**"],
    rules: {
      "no-restricted-syntax": [
        "error",
        {
          selector:
            "JSXOpeningElement[name.name='input']:not(:has(JSXAttribute[name.name='type'][value.value=/^(checkbox|radio|range|file|color|hidden)$/]))",
          message:
            "A text field is <Input> from components/ui (bare inside a <FieldShell>), #561.",
        },
        {
          selector: "JSXOpeningElement[name.name=/^(textarea|select)$/]",
          message: "Use <Textarea> or <Select> from components/ui, #561.",
        },
      ],
    },
  },
);
