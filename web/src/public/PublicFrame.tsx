// The document around every public page: <html lang>, and a <head> with
// the title, description, stylesheet, icon and the links between the
// languages. The page has no script at all. The head is written as text:
// React 19 moves <link> and <meta> to the top of <head>, which would take
// them out of the APP_PUBLIC_URL block they must stay in.

import type { ReactNode } from "react";

import type { Locale } from "../i18n/locale.ts";
import defaults from "../theme/defaults.json";
import { escapeHtml, KEY, value } from "./keys.ts";

export type FrameProps = {
  locale: Locale;
  title: string;
  description: string;
  stylesheet: string;
  /** This page's path in each language. */
  paths: Record<Locale, string>;
  /** The path search engines get for any other language, if any. */
  defaultPath?: string;
  /** A picture for link previews, as a path. */
  image?: string;
  /** Keeps search engines from listing the page (the feedback pages). */
  noindex?: boolean;
  children: ReactNode;
};

const OG_LOCALE: Record<Locale, string> = { de: "de_DE", en: "en_GB" };

function head({
  locale,
  title,
  description,
  stylesheet,
  paths,
  defaultPath,
  image,
  noindex,
}: Omit<FrameProps, "children">): string {
  const origin = value(KEY.publicUrl);
  const tag = (name: string, attributes: Record<string, string>) =>
    `<${name}${Object.entries(attributes)
      .map(([key, text]) => ` ${key}="${escapeHtml(text)}"`)
      .join("")}>`;
  const alternates = (["de", "en"] as const).map((lang) =>
    tag("link", {
      rel: "alternate",
      hreflang: lang,
      href: origin + paths[lang],
    }),
  );
  if (defaultPath)
    alternates.push(
      tag("link", {
        rel: "alternate",
        hreflang: "x-default",
        href: origin + defaultPath,
      }),
    );
  return [
    '<meta charset="utf-8">',
    tag("meta", {
      name: "viewport",
      content: "width=device-width, initial-scale=1",
    }),
    tag("meta", { name: "color-scheme", content: "dark" }),
    tag("meta", { name: "theme-color", content: defaults.dark.background }),
    `<title>${escapeHtml(title)}</title>`,
    tag("meta", { name: "description", content: description }),
    ...(noindex ? [tag("meta", { name: "robots", content: "noindex" })] : []),
    tag("link", {
      rel: "icon",
      type: "image/png",
      href: "/icons/icon-192.png",
    }),
    tag("link", { rel: "stylesheet", href: stylesheet }),
    tag("meta", { property: "og:type", content: "website" }),
    tag("meta", { property: "og:site_name", content: "Gelabber" }),
    tag("meta", { property: "og:title", content: title }),
    tag("meta", { property: "og:description", content: description }),
    tag("meta", { property: "og:locale", content: OG_LOCALE[locale] }),
    `<!--if:${KEY.publicUrl}-->`,
    tag("link", { rel: "canonical", href: origin + paths[locale] }),
    ...alternates,
    tag("meta", { property: "og:url", content: origin + paths[locale] }),
    ...(image
      ? [tag("meta", { property: "og:image", content: origin + image })]
      : []),
    `<!--end:${KEY.publicUrl}-->`,
  ].join("\n");
}

export function PublicFrame({ children, ...props }: FrameProps) {
  return (
    <html lang={props.locale}>
      <head dangerouslySetInnerHTML={{ __html: `\n${head(props)}\n` }} />
      <body>{children}</body>
    </html>
  );
}

/** The app's bubbles and the name, linking to `/`. The bubbles are the app
 * icon without its square, which would show over the hero's artwork. */
export function Brand() {
  return (
    <a className="public-brand" href="/">
      <img src="/icons/brand-mark.png" alt="" width={32} height={32} />
      {/* i18n-ok */}Gelabber
    </a>
  );
}
