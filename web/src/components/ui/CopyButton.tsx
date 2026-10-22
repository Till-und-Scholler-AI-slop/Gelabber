import type { ReactNode } from "react";

import { CheckIcon, CopyIcon } from "../Icons.tsx";
import { Button, type ButtonProps } from "./Button.tsx";
import { useCopied } from "./useCopied.ts";
import "./ui.css";

/**
 * What a copy control shows: its label and copy icon, or for a moment after
 * copying "Kopiert" and a check, crossfading in place. Both labels take the
 * same room, so nothing beside the control moves.
 */
export function CopyLabel({
  copied,
  children,
  iconSize = 16,
}: {
  copied: boolean;
  children: ReactNode;
  /** 0 for none. */
  iconSize?: number;
}) {
  return (
    <>
      {iconSize > 0 ? (
        <span className="ui-copy-icon" data-copied={copied} aria-hidden="true">
          <CopyIcon size={iconSize} />
          <CheckIcon size={iconSize} />
        </span>
      ) : null}
      <span className="ui-copy-label" data-copied={copied}>
        <span aria-hidden={copied || undefined}>{children}</span>
        <span aria-hidden={!copied || undefined}>Kopiert</span>
      </span>
    </>
  );
}

/**
 * A button that copies: `onCopy` copies and resolves whether it worked.
 * Done, the button says "Kopiert" for a moment and, unless the caller
 * reports it itself (`announce={false}`), so does a status for screen
 * readers. A failure is the caller's to explain.
 */
export function CopyButton({
  onCopy,
  announce = true,
  iconSize,
  children,
  ...button
}: Omit<ButtonProps, "onClick" | "icon" | "children"> & {
  onCopy: () => Promise<boolean>;
  announce?: boolean;
  iconSize?: number;
  children: ReactNode;
}) {
  const { copied, flash } = useCopied();
  return (
    <>
      <Button
        {...button}
        onClick={() =>
          void onCopy().then((done) => {
            if (done) flash();
          })
        }
      >
        <CopyLabel copied={copied} iconSize={iconSize}>
          {children}
        </CopyLabel>
      </Button>
      {announce ? (
        <span role="status" className="sr-only">
          {copied ? "Kopiert" : ""}
        </span>
      ) : null}
    </>
  );
}
