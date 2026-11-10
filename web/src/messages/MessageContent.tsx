import { Fragment, useEffect, useMemo, useState, type ReactNode } from "react";

import { CopyLabel } from "../components/ui/CopyButton.tsx";
import { COPIED_MS, useCopied } from "../components/ui/useCopied.ts";
import { useT } from "../i18n/catalog.ts";
import { chat } from "../i18n/catalogs/chat.ts";
import { common } from "../i18n/catalogs/common.ts";
import { copyText } from "../servers/clipboard.ts";
import { isTauriApp } from "../tauriApp.ts";
import { parseMessage, type Block, type Inline } from "./markdown.ts";
import { markMatches } from "./searchTerms.ts";

export type MentionPart =
  { type: "mention"; userId: string } | { type: "everyone" | "here" };
/** Draws a mention; without one, the raw token is shown. */
export type MentionRenderer = (part: MentionPart) => ReactNode;

/**
 * A message's text with its markdown (#250). `interactive={false}` is for
 * places that are themselves a button, like search hits: links and the copy
 * button turn into plain text there and spoilers stay hidden. `highlight`
 * marks these words (lower-cased, `searchTerms`) in the text outside
 * spoilers, code and links.
 */
export function MessageContent({
  content,
  renderMention,
  interactive = true,
  highlight = NO_TERMS,
}: {
  content: string;
  renderMention?: MentionRenderer;
  interactive?: boolean;
  highlight?: readonly string[];
}) {
  const blocks = useMemo(() => parseMessage(content), [content]);
  const ctx = { renderMention, interactive, highlight };
  return <>{blocks.map((block, n) => renderBlock(block, n, ctx))}</>;
}

const NO_TERMS: readonly string[] = [];

type Ctx = {
  renderMention?: MentionRenderer;
  interactive: boolean;
  highlight: readonly string[];
};

/** Plain text, with the searched words in `<mark>`. */
function marked(text: string, key: number, terms: readonly string[]) {
  if (!terms.length) return text;
  return (
    <Fragment key={key}>
      {markMatches(text, terms).map((run, n) =>
        run.match ? (
          <mark key={n} className="lr-search-mark">
            {run.text}
          </mark>
        ) : (
          run.text
        ),
      )}
    </Fragment>
  );
}

function renderBlock(block: Block, key: number, ctx: Ctx): ReactNode {
  switch (block.type) {
    case "paragraph":
      return <span key={key}>{renderInline(block.children, ctx)}</span>;
    case "quote":
      return ctx.interactive ? (
        <blockquote key={key} className="lr-md-quote">
          {renderInline(block.children, ctx)}
        </blockquote>
      ) : (
        <span key={key} className="lr-md-quote">
          {renderInline(block.children, ctx)}
        </span>
      );
    case "codeblock":
      return (
        <CodeBlock
          key={key}
          lang={block.lang}
          text={block.text}
          interactive={ctx.interactive}
        />
      );
  }
}

function renderInline(nodes: Inline[], ctx: Ctx): ReactNode[] {
  return nodes.map((node, key) => {
    switch (node.type) {
      case "text":
        return marked(node.text, key, ctx.highlight);
      case "strong":
        return <strong key={key}>{renderInline(node.children, ctx)}</strong>;
      case "em":
        return <em key={key}>{renderInline(node.children, ctx)}</em>;
      case "underline":
        return (
          <span key={key} className="lr-md-underline">
            {renderInline(node.children, ctx)}
          </span>
        );
      case "strike":
        return <s key={key}>{renderInline(node.children, ctx)}</s>;
      case "spoiler":
        return (
          <Spoiler key={key} interactive={ctx.interactive}>
            {renderInline(node.children, { ...ctx, highlight: NO_TERMS })}
          </Spoiler>
        );
      case "code":
        return (
          <code key={key} className="lr-md-code">
            {node.text}
          </code>
        );
      case "link":
        return ctx.interactive ? (
          <a
            key={key}
            href={node.href}
            {...newTab(node.href)}
            className="lr-md-link"
          >
            {node.text}
          </a>
        ) : (
          <span key={key} className="lr-md-link">
            {node.text}
          </span>
        );
      case "mention":
      case "everyone":
      case "here":
        return (
          <span key={key}>
            {ctx.renderMention?.(node) ??
              (node.type === "mention" ? `<@${node.userId}>` : `@${node.type}`)}
          </span>
        );
    }
  });
}

/** Web links open in a new tab. A mail link would leave an empty one in the
 * browser; the old Tauri app only hands links in new windows to the system. */
function newTab(href: string) {
  if (/^https?:/i.test(href))
    return { target: "_blank", rel: "noopener noreferrer nofollow" };
  return isTauriApp() ? { target: "_blank", rel: "noopener noreferrer" } : {};
}

function Spoiler({
  interactive,
  children,
}: {
  interactive: boolean;
  children: ReactNode;
}) {
  const t = useT(chat);
  const [shown, setShown] = useState(false);
  if (!interactive) return <span className="lr-md-spoiler">{children}</span>;
  // One element before and after revealing: it keeps the keyboard focus
  // (tabIndex -1) instead of dropping it to the page.
  return (
    <span
      className={`lr-md-spoiler${shown ? " lr-md-spoiler-shown" : ""}`}
      role={shown ? undefined : "button"}
      tabIndex={shown ? -1 : 0}
      aria-label={shown ? undefined : t("content.showSpoilerLabel")}
      onClick={(event) => {
        if (shown) return;
        event.stopPropagation();
        setShown(true);
      }}
      onKeyDown={(event) => {
        if (!shown && (event.key === "Enter" || event.key === " ")) {
          event.preventDefault();
          setShown(true);
        }
      }}
    >
      {shown ? (
        children
      ) : (
        // Links inside stay out of reach, by keyboard too, until revealed.
        <span aria-hidden inert>
          {children}
        </span>
      )}
    </span>
  );
}

function CodeBlock({
  lang,
  text,
  interactive,
}: {
  lang: string | null;
  text: string;
  interactive: boolean;
}) {
  const t = useT(chat);
  const tc = useT(common);
  const { copied, flash } = useCopied();
  const [failed, setFailed] = useState(false);
  useEffect(() => {
    if (!failed) return;
    const timer = window.setTimeout(() => setFailed(false), COPIED_MS);
    return () => window.clearTimeout(timer);
  }, [failed]);
  if (!interactive)
    return (
      <span className="lr-md-codeblock">
        <code>{text}</code>
      </span>
    );
  return (
    <div className="lr-md-codeblock">
      <div className="lr-md-codeblock-bar">
        <span>{lang ?? ""}</span>
        <button
          type="button"
          onClick={() =>
            // The clipboard is missing outside a secure context and in
            // some web views.
            void copyText(text).then((done) => {
              setFailed(!done);
              if (done) flash();
            })
          }
        >
          {failed ? (
            t("content.copyFailed")
          ) : (
            <CopyLabel copied={copied} iconSize={13}>
              {tc("copy")}
            </CopyLabel>
          )}
        </button>
      </div>
      <pre>
        <code data-lang={lang ?? undefined}>{text}</code>
      </pre>
    </div>
  );
}
