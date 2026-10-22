import type { ReactNode } from "react";

import "./ui.css";

/**
 * Placeholder shapes while something loads. They fade in only after 150ms,
 * so a fast answer never flashes them. The shapes are hidden from assistive
 * technology, which hears "Wird geladen…" instead.
 */
function SkeletonRegion({
  className,
  children,
}: {
  className?: string;
  children: ReactNode;
}) {
  return (
    <div className={["ui-skeleton", className].filter(Boolean).join(" ")}>
      <span className="sr-only">Wird geladen…</span>
      <div aria-hidden="true" className="contents">
        {children}
      </div>
    </div>
  );
}

const LINE_WIDTHS = ["38%", "86%", "62%", "74%", "48%", "92%"];

function Rows({ count, avatars }: { count: number; avatars: boolean }) {
  return (
    <div className="ui-skeleton-rows">
      {Array.from({ length: count }, (_, index) => (
        <div key={index} className="ui-skeleton-row">
          {avatars ? (
            <span className="ui-skeleton-block ui-skeleton-avatar" />
          ) : null}
          <div>
            <span
              className="ui-skeleton-block ui-skeleton-line"
              style={{ width: avatars ? "22%" : LINE_WIDTHS[index % 6] }}
            />
            {avatars ? (
              <span
                className="ui-skeleton-block ui-skeleton-line"
                style={{ width: LINE_WIDTHS[(index + 1) % 6] }}
              />
            ) : null}
          </div>
        </div>
      ))}
    </div>
  );
}

function Card() {
  return (
    <div className="ui-skeleton-card">
      <span
        className="ui-skeleton-block ui-skeleton-line"
        style={{ width: "45%", height: 14 }}
      />
      <span
        className="ui-skeleton-block ui-skeleton-line"
        style={{ width: "90%" }}
      />
      <span
        className="ui-skeleton-block ui-skeleton-line"
        style={{ width: "70%" }}
      />
    </div>
  );
}

/** List rows: with `avatars`, a circle and two lines like a message or member. */
export function SkeletonRows({
  count = 4,
  avatars = false,
  className,
}: {
  count?: number;
  avatars?: boolean;
  className?: string;
}) {
  return (
    <SkeletonRegion className={className}>
      <Rows count={count} avatars={avatars} />
    </SkeletonRegion>
  );
}

export function SkeletonCard({ className }: { className?: string }) {
  return (
    <SkeletonRegion className={className}>
      <Card />
    </SkeletonRegion>
  );
}

/** Fills the main column while a page loads: a chat, an overview or a list. */
export function MainSkeleton({ kind }: { kind: "chat" | "overview" | "list" }) {
  return (
    <SkeletonRegion className={`ui-main-skeleton ui-main-skeleton--${kind}`}>
      {kind === "chat" ? <Rows count={6} avatars /> : null}
      {kind === "list" ? <Rows count={8} avatars={false} /> : null}
      {kind === "overview" ? (
        <>
          <span
            className="ui-skeleton-block"
            style={{ width: "min(420px, 70%)", height: 32 }}
          />
          <div className="ui-main-skeleton__cards">
            <Card />
            <Card />
          </div>
        </>
      ) : null}
    </SkeletonRegion>
  );
}
