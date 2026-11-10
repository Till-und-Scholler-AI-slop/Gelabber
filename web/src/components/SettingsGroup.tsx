import type { ReactNode } from "react";

import { useT } from "../i18n/catalog.ts";
import { common } from "../i18n/catalogs/common.ts";
import type { SettingsScope } from "./settingsScope.ts";
import { Badge } from "./ui/Badge.tsx";

/**
 * A titled group of settings. The tag says where they are kept: on this
 * device only, or with the account on every device. A quiet tag: the
 * accent is for the page's actions.
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
  const t = useT(common);
  return <Badge>{t(`scope.${scope}`)}</Badge>;
}
