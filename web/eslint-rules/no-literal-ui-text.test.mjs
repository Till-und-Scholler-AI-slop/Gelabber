// The hardcoded-string check that `npm run lint` runs (eslint.config.js).
import { RuleTester } from "eslint";
import { describe, it } from "vitest";

import rule from "./no-literal-ui-text.js";

RuleTester.describe = describe;
RuleTester.it = it;
RuleTester.itOnly = it.only;

const tester = new RuleTester({
  languageOptions: {
    ecmaVersion: "latest",
    sourceType: "module",
    parserOptions: { ecmaFeatures: { jsx: true } },
  },
});

const error = [{ messageId: "literal" }];

tester.run("no-literal-ui-text", rule, {
  valid: [
    "const a = <p>{t('hint')}</p>;",
    "const a = <p> · </p>;",
    "const a = <p>{' – '}</p>;",
    'const a = <img alt="" src="/x.png" />;',
    'const a = <Button aria-label={t("close")} />;',
    'const a = <div className="Some Class Names" />;',
    "const a = <strong>Gelabber</strong>; // i18n-ok",
    "const a = (\n  <strong>\n    Gelabber {/* i18n-ok */}\n  </strong>\n);",
    'const o = { id: "dark", label: "dark" };',
    'const o = { label: t("theme.dark") };',
    'const o = { name: "Gelabber Dunkel" };',
    'const o = { label: "Gelabber Dunkel" }; // i18n-ok',
    'const o = { text: "ok" };',
  ],
  invalid: [
    { code: "const a = <p>Verbinde…</p>;", errors: error },
    { code: "const a = <button>Speichern</button>;", errors: error },
    { code: 'const a = <Button aria-label="Schließen" />;', errors: error },
    { code: 'const a = <input placeholder="Suchen" />;', errors: error },
    { code: "const a = <img alt={`Avatar`} />;", errors: error },
    {
      code: 'const a = <Switch label="Bewegung reduzieren" />;',
      errors: error,
    },
    { code: 'const o = { label: "Groß" };', errors: error },
    { code: 'const o = { message: "Nicht gespeichert" };', errors: error },
    { code: 'const o = { "error": "Bitte erneut versuchen" };', errors: error },
    { code: "const o = { hint: `Gilt für ${n} Geräte` };", errors: error },
    {
      // The comment exempts its own line only.
      code: "const a = (\n  <p>\n    Hallo\n  </p>\n); // i18n-ok",
      errors: error,
    },
  ],
});
