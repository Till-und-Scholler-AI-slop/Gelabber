import { renderToStaticMarkup } from "react-dom/server";
import { afterEach, describe, expect, it, vi } from "vitest";

import { PresenceSetting } from "./PresenceSetting.tsx";

afterEach(() => vi.unstubAllGlobals());

describe("presence setting", () => {
  it("explains the fifteen minutes where the browser cannot see other programs", () => {
    vi.stubGlobal("IdleDetector", undefined);
    const html = renderToStaticMarkup(<PresenceSetting />);
    expect(html).toContain(
      "Browser sehen keine Eingaben in anderen Programmen.",
    );
    expect(html).toContain("15 Minuten");
    expect(html).not.toContain('role="switch"');
  });

  it("offers the switch where the browser has idle detection", () => {
    vi.stubGlobal("IdleDetector", class {});
    const html = renderToStaticMarkup(<PresenceSetting />);
    expect(html).toContain('role="switch"');
    expect(html).toContain("Eingaben am ganzen Computer zählen");
    expect(html).toContain("erst nach 15 Minuten");
  });
});
