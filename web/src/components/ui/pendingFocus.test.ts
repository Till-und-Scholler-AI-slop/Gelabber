import { describe, expect, it } from "vitest";

import { pendingFocus } from "./pendingFocus.ts";

function page() {
  const document = { activeElement: null as unknown, body: {} };
  document.activeElement = document.body;
  const button = {
    disabled: false,
    isConnected: true,
    inClosedDialog: false,
    ownerDocument: document,
    closest: (selector: string) =>
      selector === "dialog:not([open])" && button.inClosedDialog ? {} : null,
    focus: () => {
      document.activeElement = button;
    },
  };
  // What the browser does when the focused button turns pending.
  const disable = () => {
    button.disabled = true;
    document.activeElement = document.body;
  };
  return { document, button, disable };
}

describe("pending focus", () => {
  it("gives focus back to the button once its request is over", () => {
    const { document, button, disable } = page();
    const memory = pendingFocus();
    button.focus();
    memory.focused(button);
    disable();
    memory.blurred(button);
    button.disabled = false;
    memory.settled();
    expect(document.activeElement).toBe(button);
  });

  it("leaves focus where the user moved it meanwhile", () => {
    const { document, button, disable } = page();
    const memory = pendingFocus();
    button.focus();
    memory.focused(button);
    disable();
    const field = {};
    document.activeElement = field;
    memory.settled();
    expect(document.activeElement).toBe(field);
  });

  it("does not reach into a dialog that has closed", () => {
    const { document, button, disable } = page();
    const memory = pendingFocus();
    button.focus();
    memory.focused(button);
    disable();
    button.inClosedDialog = true;
    memory.settled();
    expect(document.activeElement).toBe(document.body);
  });

  it("forgets the button once the user tabs away from it", () => {
    const { document, button } = page();
    const memory = pendingFocus();
    button.focus();
    memory.focused(button);
    document.activeElement = document.body;
    memory.blurred(button);
    memory.settled();
    expect(document.activeElement).toBe(document.body);
  });
});
