import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";

import { PasswordField } from "./PasswordField.tsx";
import { passwordToggle } from "./passwordToggle.ts";

describe("password field", () => {
  it("starts hidden, with a named, unpressed toggle", () => {
    const html = renderToStaticMarkup(
      <PasswordField
        id="password"
        label="Passwort"
        autoComplete="current-password"
      />,
    );
    expect(html).toContain('type="password"');
    expect(html).toContain('autoComplete="current-password"');
    expect(html).toMatch(
      /<button[^>]*aria-label="Passwort anzeigen"[^>]*aria-pressed="false"/,
    );
    expect(html).toContain('type="button"');
  });

  it("flips the input type and aria-pressed with the toggle", () => {
    expect(passwordToggle(false)).toEqual({
      type: "password",
      pressed: false,
      title: "Passwort anzeigen",
    });
    expect(passwordToggle(true)).toEqual({
      type: "text",
      pressed: true,
      title: "Passwort verbergen",
    });
  });

  it("describes the input by its error, without a second alert", () => {
    const html = renderToStaticMarkup(
      <PasswordField id="pw" label="Passwort" error="Passwort fehlt." />,
    );
    expect(html).toContain('aria-invalid="true"');
    expect(html).toContain('aria-describedby="pw-error"');
    expect(html).toContain('<p id="pw-error" class="ui-field-error">');
    expect(html).not.toContain('role="alert"');
  });
});
