import { describe, expect, it, vi } from "vitest";

import { fieldSummary, focusFirstInvalid } from "./useFormErrors.ts";

describe("form errors", () => {
  it("announces several field errors as one summary", () => {
    expect(fieldSummary({})).toBeNull();
    // One error: focus moves to its field, which reads it out.
    expect(fieldSummary({ email: "E-Mail fehlt." })).toBeNull();
    expect(fieldSummary({ name: "Name fehlt.", email: "E-Mail fehlt." })).toBe(
      "Bitte die markierten Felder prüfen.",
    );
  });

  it("moves focus to the first invalid field", () => {
    const focus = vi.fn();
    const querySelector = vi.fn(() => ({ focus }));
    focusFirstInvalid({ querySelector } as unknown as ParentNode);
    expect(querySelector).toHaveBeenCalledWith('[aria-invalid="true"]');
    expect(focus).toHaveBeenCalledOnce();
    expect(() =>
      focusFirstInvalid({ querySelector: () => null } as unknown as ParentNode),
    ).not.toThrow();
    expect(() => focusFirstInvalid(null)).not.toThrow();
  });
});
