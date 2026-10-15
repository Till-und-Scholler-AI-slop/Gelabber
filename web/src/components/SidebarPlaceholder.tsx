import { UserPanel } from "./UserPanel.tsx";

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
 */
export function SidebarPlaceholder({ failure }: { failure?: SidebarFailure }) {
  return (
    <aside
      className="channel-sidebar"
      aria-label={failure ? "Navigation" : "Navigation wird geladen"}
    >
      <div className="sidebar-brand">Gelabber</div>
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
              {failure.pending ? "Wird geladen…" : "Erneut versuchen"}
            </button>
          </>
        ) : null}
      </div>
      <UserPanel />
    </aside>
  );
}
