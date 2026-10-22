import type { ReactNode } from "react";

import { SCOPE_LABEL, type SettingsScope } from "./settingsScope.ts";
import { Badge } from "./ui/Badge.tsx";

/**
 * A titled group of settings. The tag says where they are kept: on this
 * device only, or with the account on every device.
 */
export function SettingsGroup({
  title,
  scope,
  children,
}: {
  title: ReactNode;
  scope?: SettingsScope;
  children: ReactNode;
}) {
  return (
    <fieldset className="settings-group">
      <legend className="settings-group-legend">
        <span>{title}</span>
        {scope ? <ScopeTag scope={scope} /> : null}
      </legend>
      {children}
    </fieldset>
  );
}

export function ScopeTag({ scope }: { scope: SettingsScope }) {
  return (
    <Badge tone={scope === "account" ? "accent" : "neutral"}>
      {SCOPE_LABEL[scope]}
    </Badge>
  );
}
