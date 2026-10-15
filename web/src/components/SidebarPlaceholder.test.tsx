import { isValidElement, type ReactElement, type ReactNode } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it, vi } from "vitest";

vi.mock("./UserPanel.tsx", () => ({ UserPanel: () => null }));

import { SidebarPlaceholder } from "./SidebarPlaceholder.tsx";

type Props = { children?: ReactNode; onClick?: () => void; type?: string };

/** The `<button>` in an element tree, without rendering it. */
function findButton(node: ReactNode): ReactElement<Props> | undefined {
  if (Array.isArray(node)) {
    for (const child of node) {
      const found = findButton(child);
      if (found) return found;
    }
    return undefined;
  }
  if (!isValidElement<Props>(node)) return undefined;
  if (node.type === "button") return node;
  return findButton(node.props.children);
}

describe("sidebar placeholder", () => {
  it("keeps an empty status region while loading", () => {
    const html = renderToStaticMarkup(<SidebarPlaceholder />);
    expect(html).toContain('aria-label="Navigation wird geladen"');
    expect(html).toContain('<div role="status"></div>');
    expect(html).toContain('class="sidebar-skeleton" aria-hidden="true"');
    expect(html).not.toContain("<button");
  });

  it("puts a failure into that status region with a retry", () => {
    const html = renderToStaticMarkup(
      <SidebarPlaceholder
        failure={{
          message: "Die Kanäle konnten gerade nicht geladen werden.",
          pending: false,
          onRetry: () => {},
        }}
      />,
    );
    expect(html).toContain('aria-label="Navigation"');
    expect(html).toMatch(
      /<div role="status"[^>]*><p>Die Kanäle konnten gerade nicht geladen werden\.<\/p><button type="button">Erneut versuchen<\/button>/,
    );
    expect(html).not.toContain("sidebar-skeleton");
  });

  it("disables the retry while it runs", () => {
    const html = renderToStaticMarkup(
      <SidebarPlaceholder
        failure={{ message: "Fehler", pending: true, onRetry: () => {} }}
      />,
    );
    expect(html).toContain('<button type="button" disabled="">Wird geladen…');
  });

  it("calls the retry on click", () => {
    const onRetry = vi.fn();
    const button = findButton(
      SidebarPlaceholder({
        failure: { message: "Fehler", pending: false, onRetry },
      }),
    );
    button?.props.onClick?.();
    expect(onRetry).toHaveBeenCalledOnce();
  });
});
