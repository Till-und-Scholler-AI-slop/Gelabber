// String catalogs: one namespace per file in ./catalogs, German and English
// side by side. The types make a missing or extra English key, a plural
// that is not a plural in both, and a missing `{placeholder}` a compile
// error; catalogs.test.ts checks what types cannot (same placeholders and
// tags in both languages, no empty text). docs/i18n.md has the rules.

import { createElement, Fragment, useCallback, type ReactNode } from "react";

import {
  currentLocale,
  intlLocale,
  useCurrentLocale,
  type Locale,
} from "./locale.ts";

/** A text that depends on a count; `count` picks the form. */
export type Plural = { readonly one: string; readonly other: string };
export type Entry = string | Plural;
export type Entries = { readonly [key: string]: Entry };

/** The English side of a catalog: exactly the German keys, plural for plural. */
export type Translation<De extends Entries> = {
  readonly [K in keyof De]: De[K] extends string ? string : Plural;
};

export type Catalog<De extends Entries = Entries> = {
  readonly ns: string;
  readonly de: De;
  readonly en: Translation<De>;
};

type Texts<E> = E extends Plural ? E["one"] | E["other"] : E;

/** `{name}` placeholders of a text, as a union of their names. */
type Placeholders<S> = S extends `${string}{${infer P}}${infer Rest}`
  ? P | Placeholders<Rest>
  : never;

/** `<tag>…</tag>` names of a text. */
type TagNames<S> = S extends `${string}<${infer N}>${infer Rest}`
  ? N extends `/${string}`
    ? TagNames<Rest>
    : N | TagNames<Rest>
  : never;

export type ParamValue = string | number;

/** What a text needs filled in; a plural needs `count`. */
export type Params<E> = E extends Plural
  ? { count: number } & {
      [P in Exclude<Placeholders<Texts<E>>, "count">]: ParamValue;
    }
  : { [P in Placeholders<E>]: ParamValue };

type Args<E> = E extends Plural
  ? [params: Params<E>]
  : [Placeholders<E>] extends [never]
    ? []
    : [params: Params<E>];

/**
 * Declares a namespace. `ns` is the file name; English must have exactly
 * the German keys.
 */
export function defineCatalog<const De extends Entries>(
  ns: string,
  de: De,
  en: NoInfer<Translation<De>>,
): Catalog<De> {
  return { ns, de, en };
}

const pluralRules = new Map<Locale, Intl.PluralRules>();
const numbers = new Map<Locale, Intl.NumberFormat>();

function pluralRule(locale: Locale): Intl.PluralRules {
  let rules = pluralRules.get(locale);
  if (!rules) {
    rules = new Intl.PluralRules(intlLocale(locale));
    pluralRules.set(locale, rules);
  }
  return rules;
}

function formatParam(locale: Locale, value: ParamValue): string {
  if (typeof value === "string") return value;
  let format = numbers.get(locale);
  if (!format) {
    format = new Intl.NumberFormat(intlLocale(locale));
    numbers.set(locale, format);
  }
  return format.format(value);
}

/** The entry, else (only through a cast) the German one, reported in dev. */
function entryOf(catalog: Catalog, locale: Locale, key: string): Entry {
  const entry = (catalog[locale] as Entries)[key];
  if (entry !== undefined) return entry;
  if (import.meta.env?.DEV)
    console.error(`i18n: "${catalog.ns}" has no "${key}" in ${locale}`);
  return catalog.de[key] ?? key;
}

function textOf(
  catalog: Catalog,
  locale: Locale,
  key: string,
  params: Record<string, ParamValue> | undefined,
): string {
  const entry = entryOf(catalog, locale, key);
  if (typeof entry === "string") return entry;
  const count = params?.count;
  return typeof count === "number" && pluralRule(locale).select(count) === "one"
    ? entry.one
    : entry.other;
}

function interpolate(
  locale: Locale,
  text: string,
  params: Record<string, ParamValue> | undefined,
): string {
  if (!params) return text;
  return text.replace(/\{([a-zA-Z0-9_]+)\}/g, (match, name: string) =>
    Object.hasOwn(params, name) ? formatParam(locale, params[name]) : match,
  );
}

function textIn(
  locale: Locale,
  catalog: Catalog,
  key: string,
  params: Record<string, ParamValue> | undefined,
): string {
  return interpolate(locale, textOf(catalog, locale, key, params), params);
}

/** A text in a given language. Most code wants `translate` or `useT`. */
export function translateIn<De extends Entries, K extends keyof De & string>(
  locale: Locale,
  catalog: Catalog<De>,
  key: K,
  ...args: Args<De[K]>
): string {
  return textIn(
    locale,
    catalog as Catalog,
    key,
    args[0] as Record<string, ParamValue> | undefined,
  );
}

/** A text in the interface's language now. Outside React, or for a one-off. */
export function translate<De extends Entries, K extends keyof De & string>(
  catalog: Catalog<De>,
  key: K,
  ...args: Args<De[K]>
): string {
  return translateIn(currentLocale(), catalog, key, ...args);
}

export type TFunction<De extends Entries> = <K extends keyof De & string>(
  key: K,
  ...args: Args<De[K]>
) => string;

/** `t(key, params)` for one namespace; renders again when the language changes. */
export function useT<De extends Entries>(catalog: Catalog<De>): TFunction<De> {
  const locale = useCurrentLocale();
  return useCallback(
    (key: string, ...args: unknown[]) =>
      textIn(
        locale,
        catalog as Catalog,
        key,
        args[0] as Record<string, ParamValue> | undefined,
      ),
    [locale, catalog],
  ) as TFunction<De>;
}

export type RichPiece = string | { tag: string; text: string };

/** Splits `a <b>b</b> c` into text and tagged pieces; tags do not nest. */
export function richPieces(text: string): RichPiece[] {
  const pieces: RichPiece[] = [];
  let at = 0;
  for (const match of text.matchAll(/<([a-zA-Z][a-zA-Z0-9]*)>(.*?)<\/\1>/gs)) {
    if (match.index > at) pieces.push(text.slice(at, match.index));
    pieces.push({ tag: match[1], text: match[2] });
    at = match.index + match[0].length;
  }
  if (at < text.length) pieces.push(text.slice(at));
  return pieces;
}

type ParamsProp<E> =
  Args<E> extends [] ? { params?: undefined } : { params: Params<E> };

type TagsProp<E> = [TagNames<Texts<E>>] extends [never]
  ? { tags?: undefined }
  : { tags: { [N in TagNames<Texts<E>>]: (text: string) => ReactNode } };

export type TProps<De extends Entries, K extends keyof De & string> = {
  catalog: Catalog<De>;
  k: K;
} & ParamsProp<De[K]> &
  TagsProp<De[K]>;

/**
 * Rich text: `<link>…</link>` and `<b>…</b>` in the catalog become what
 * `tags` renders for them; placeholders are filled in each piece.
 */
export function T<De extends Entries, K extends keyof De & string>(
  props: TProps<De, K>,
): ReactNode {
  const locale = useCurrentLocale();
  const params = props.params as Record<string, ParamValue> | undefined;
  const tags = (props.tags ?? {}) as Record<
    string,
    ((text: string) => ReactNode) | undefined
  >;
  const pieces = richPieces(
    textOf(props.catalog as Catalog, locale, props.k, params),
  );
  return createElement(
    Fragment,
    null,
    ...pieces.map((piece) => {
      if (typeof piece === "string") return interpolate(locale, piece, params);
      const text = interpolate(locale, piece.text, params);
      const render = tags[piece.tag];
      return render ? render(text) : text;
    }),
  );
}
