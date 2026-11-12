// The public pages' texts in one fixed language: they are written out at
// build time, once per language, so they never follow the app's language.

import { translateIn } from "../i18n/catalog.ts";
import { public as publicPages } from "../i18n/catalogs/public.ts";
import type { Locale } from "../i18n/locale.ts";

type Texts = typeof publicPages.de;

/** The keys whose text needs nothing filled in. */
export type PlainKey = {
  [K in keyof Texts]: Texts[K] extends `${string}{${string}}${string}`
    ? never
    : K;
}[keyof Texts] &
  string;

export function textsIn(locale: Locale): (key: PlainKey) => string {
  return (key) => translateIn(locale, publicPages, key);
}

/** The three commands of README.md "Start", the same in both languages. */
export const SELF_HOST_COMMANDS = [
  "cp deploy/compose/.env.example deploy/compose/.env",
  "cd deploy/compose",
  "docker compose up",
] as const;
