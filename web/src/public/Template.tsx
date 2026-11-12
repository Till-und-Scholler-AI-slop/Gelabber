// The blocks of a public page's template (keys.ts). React cannot write
// comments, so these render as stand-in elements that render.tsx turns
// into the template's marker lines.

import { createElement, type ReactNode } from "react";

import type { TemplateKey } from "./keys.ts";

/** Kept only when the container has a value for `when`. */
export function If({
  when,
  children,
}: {
  when: TemplateKey;
  children: ReactNode;
}) {
  return createElement("gelabber-if", { "data-key": when }, children);
}

/** Where the operator's own text for a page goes, as it is. */
export function FragmentSlot({ name }: { name: string }) {
  return createElement("gelabber-fragment", { "data-name": name });
}
