import { UserPanel } from "./UserPanel.tsx";
import { translate } from "../i18n/catalog.ts";
import { shell } from "../i18n/catalogs/shell.ts";
import { common } from "../i18n/catalogs/common.ts";

export type SidebarFailure = {
  message: string;
  pending: boolean;
  onRetry: () => void;
};

/**
 * The sidebar while its list loads, or after it failed. One element for
 * both, so the status region already exists when the failure text arrives:
 * screen readers announce a change in a live region, not a new one. Not
 * every page beside it reports the failure (a cached DM thread does not).
 * No hooks of its own (tests call it as a function): the page around it
 * renders again when the language changes.
 */
export function SidebarPlaceholder({ failure }: { failure?: SidebarFailure }) {
  return (
    <aside
      className="channel-sidebar"
      aria-label={translate(
        shell,
        failure ? "sidebar.label" : "sidebar.loadingLabel",
      )}
    >
      <div className="sidebar-brand">Gelabber{/* i18n-ok */}</div>
      {failure ? null : (
        <div className="sidebar-skeleton" aria-hidden>
          {[0, 1, 2, 3].map((index) => (
            <div key={index} className="animate-pulse" />
          ))}
        </div>
      )}
      <div
        role="status"
        className={failure ? "sidebar-empty sidebar-failed" : undefined}
      >
        {failure ? (
          <>
            <p>{failure.message}</p>
            <button
              type="button"
              disabled={failure.pending}
              onClick={failure.onRetry}
            >
              {translate(common, failure.pending ? "loading" : "tryAgain")}
            </button>
          </>
        ) : null}
      </div>
      <UserPanel />
    </aside>
  );
}
