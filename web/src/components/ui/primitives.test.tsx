import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";

import { Button, GhostButton } from "./Button.tsx";
import { buttonClass } from "./buttonClass.ts";
import { nextChipIndex } from "./chipKeys.ts";
import { ChipGroup } from "./ChipGroup.tsx";
import { fieldDescription } from "./fieldDescription.ts";
import { IconButton } from "./IconButton.tsx";
import { InlineStatus } from "./InlineStatus.tsx";
import { popoverPosition } from "./popoverPosition.ts";
import { MainSkeleton, SkeletonRows } from "./Skeleton.tsx";
import { Switch } from "./Switch.tsx";

describe("button", () => {
  it("swaps its label and reports busy while pending", () => {
    const html = renderToStaticMarkup(
      <Button variant="primary" pending pendingLabel="Speichert…">
        Speichern
      </Button>,
    );
    expect(html).toContain("Speichert…");
    expect(html).not.toContain("Speichern<");
    expect(html).toContain('aria-busy="true"');
    expect(html).toContain("disabled");
    expect(html).toContain("ui-button--primary");
  });

  it("is a plain button unless asked to submit", () => {
    expect(renderToStaticMarkup(<Button>Abbrechen</Button>)).toContain(
      'type="button"',
    );
  });

  it("marks the control a dialog opens on", () => {
    expect(
      renderToStaticMarkup(<GhostButton initialFocus>Fertig</GhostButton>),
    ).toContain("autofocus");
  });

  it("lends its look to links", () => {
    expect(buttonClass({ variant: "danger", size: "lg" })).toBe(
      "ui-button ui-button--danger ui-button--lg",
    );
  });
});

describe("icon button", () => {
  it("is named by its label and shows it as a tooltip", () => {
    const html = renderToStaticMarkup(
      <IconButton label="Schließen" icon={<svg />} />,
    );
    expect(html).toContain('aria-label="Schließen"');
    expect(html).toContain('title="Schließen"');
    expect(html).not.toContain("aria-pressed");
  });

  it("keeps a fixed name for toggles and reports the state", () => {
    const html = renderToStaticMarkup(
      <IconButton
        label="Stummschalten"
        title="Stummschalten (Strg+Umschalt+M)"
        pressed
        icon={<svg />}
      />,
    );
    expect(html).toContain('aria-label="Stummschalten"');
    expect(html).toContain('aria-pressed="true"');
    expect(html).toContain('title="Stummschalten (Strg+Umschalt+M)"');
  });
});

describe("fields", () => {
  it("points at the error first, else the hint, plus anything extra", () => {
    expect(fieldDescription("a", { error: "x", hint: "y" })).toBe("a-error");
    expect(fieldDescription("a", { hint: "y" })).toBe("a-hint");
    expect(fieldDescription("a", { hint: "y", extra: "a-caps" })).toBe(
      "a-hint a-caps",
    );
    expect(fieldDescription("a", {})).toBeUndefined();
  });

  it("makes the whole row of a labelled switch the target", () => {
    const html = renderToStaticMarkup(
      <Switch label="Ton beim Beitreten" description="Nur auf diesem Gerät" />,
    );
    expect(html).toMatch(/^<label class="ui-choice-row/);
    expect(html).toContain('role="switch"');
    expect(html).toContain("<small>Nur auf diesem Gerät</small>");
  });
});

describe("chip group", () => {
  const disabled = [false, true, false, false];

  it("moves with the arrows, skips disabled chips and wraps", () => {
    expect(nextChipIndex("ArrowRight", 0, disabled)).toBe(2);
    expect(nextChipIndex("ArrowLeft", 2, disabled)).toBe(0);
    expect(nextChipIndex("ArrowRight", 3, disabled)).toBe(0);
    expect(nextChipIndex("ArrowUp", 0, disabled)).toBe(3);
    expect(nextChipIndex("Home", 3, disabled)).toBe(0);
    expect(nextChipIndex("End", 0, disabled)).toBe(3);
    expect(nextChipIndex("Enter", 0, disabled)).toBeNull();
  });

  it("is a radio group with one tab stop on the chosen chip", () => {
    const html = renderToStaticMarkup(
      <ChipGroup
        label="Gültig für"
        value="7d"
        onChange={() => {}}
        options={[
          { value: "1h", label: "1 Stunde" },
          { value: "7d", label: "7 Tage" },
        ]}
      />,
    );
    expect(html).toContain('role="radiogroup" aria-label="Gültig für"');
    expect(html).toMatch(
      /role="radio" aria-checked="false" tabindex="-1"[^>]*>1 Stunde/,
    );
    expect(html).toMatch(
      /role="radio" aria-checked="true" tabindex="0"[^>]*>7 Tage/,
    );
  });
});

describe("inline status", () => {
  it("is a mounted live region even while empty", () => {
    expect(renderToStaticMarkup(<InlineStatus status={null} />)).toBe(
      '<span role="status" class="ui-inline-status"></span>',
    );
    expect(
      renderToStaticMarkup(
        <InlineStatus status={{ tone: "success", message: "Gespeichert." }} />,
      ),
    ).toContain("Gespeichert.");
  });
});

describe("skeletons", () => {
  it("hide their shapes and say that something loads", () => {
    for (const html of [
      renderToStaticMarkup(<SkeletonRows count={2} avatars />),
      renderToStaticMarkup(<MainSkeleton kind="overview" />),
    ]) {
      expect(html).toContain('<span class="sr-only">Wird geladen…</span>');
      expect(html).toContain('aria-hidden="true"');
    }
  });
});

describe("popover placement", () => {
  const viewport = { width: 1000, height: 800 };
  const size = { width: 200, height: 150 };
  const anchor = { top: 100, right: 340, bottom: 132, left: 300 };

  it("opens below, aligned to the anchor's start or end", () => {
    expect(popoverPosition(anchor, size, viewport, "bottom-start")).toEqual({
      top: 138,
      left: 300,
    });
    expect(popoverPosition(anchor, size, viewport, "bottom-end")).toEqual({
      top: 138,
      left: 140,
    });
  });

  it("flips above when there is no room below, and back", () => {
    const low = { top: 700, right: 340, bottom: 732, left: 300 };
    expect(popoverPosition(low, size, viewport, "bottom-start").top).toBe(544);
    expect(popoverPosition(anchor, size, viewport, "top-start").top).toBe(138);
  });

  it("stays inside the viewport", () => {
    const edge = { top: 100, right: 990, bottom: 132, left: 950 };
    expect(popoverPosition(edge, size, viewport, "bottom-start").left).toBe(
      792,
    );
  });

  it("places in the page's pixels when the Textgröße zooms it", () => {
    // At 1.25 the 800px window is 640 page pixels tall, and an anchor at
    // 750–790 screen pixels sits at 600–632.
    const low = { top: 750, right: 340, bottom: 790, left: 300 };
    expect(popoverPosition(low, size, viewport, "top-start", 1.25)).toEqual({
      top: 600 - 6 - 150,
      left: 240,
    });
    expect(popoverPosition(low, size, viewport, "bottom-start", 1.25)).toEqual({
      top: 600 - 6 - 150,
      left: 240,
    });
  });
});
