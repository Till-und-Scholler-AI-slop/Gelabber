import { useMemo, useState, type ReactNode } from "react";

import { parseMessage, type Block, type Inline } from "./markdown.ts";

export type MentionPart =
  { type: "mention"; userId: string } | { type: "everyone" | "here" };
/** Draws a mention; without one, the raw token is shown. */
export type MentionRenderer = (part: MentionPart) => ReactNode;

/**
 * A message's text with its markdown (#250). `interactive={false}` is for
 * places that are themselves a button, like search hits: links and the copy
 * button turn into plain text there and spoilers stay hidden.
 */
export function MessageContent({
  content,
  renderMention,
  interactive = true,
}: {
  content: string;
  renderMention?: MentionRenderer;
  interactive?: boolean;
}) {
  const blocks = useMemo(() => parseMessage(content), [content]);
  const ctx = { renderMention, interactive };
  return <>{blocks.map((block, n) => renderBlock(block, n, ctx))}</>;
}

type Ctx = { renderMention?: MentionRenderer; interactive: boolean };

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
        return node.text;
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
            {renderInline(node.children, ctx)}
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
            target="_blank"
            rel="noopener noreferrer nofollow"
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

function Spoiler({
  interactive,
  children,
}: {
  interactive: boolean;
  children: ReactNode;
}) {
  const [shown, setShown] = useState(false);
  if (!interactive) return <span className="lr-md-spoiler">{children}</span>;
  if (shown)
    return (
      <span className="lr-md-spoiler lr-md-spoiler-shown">{children}</span>
    );
  return (
    <span
      className="lr-md-spoiler"
      role="button"
      tabIndex={0}
      aria-label="Spoiler anzeigen"
      onClick={(event) => {
        event.stopPropagation();
        setShown(true);
      }}
      onKeyDown={(event) => {
        if (event.key === "Enter" || event.key === " ") {
          event.preventDefault();
          setShown(true);
        }
      }}
    >
      <span aria-hidden>{children}</span>
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
  const [copied, setCopied] = useState(false);
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
          onClick={() => {
            void navigator.clipboard?.writeText(text).then(
              () => {
                setCopied(true);
                window.setTimeout(() => setCopied(false), 1500);
              },
              () => {},
            );
          }}
        >
          {copied ? "Kopiert" : "Kopieren"}
        </button>
      </div>
      <pre>
        <code data-lang={lang ?? undefined}>{text}</code>
      </pre>
    </div>
  );
}
