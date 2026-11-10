import { useRouter } from "@tanstack/react-router";
import { useEffect, useState, type ReactNode } from "react";

import {
  logout,
  retrySession,
  useSession,
  useSessionProblems,
} from "../auth/session.ts";
import { useCooldown } from "../auth/useCooldown.ts";
import { AuthFrame } from "../pages/AuthLayout.tsx";
import { Button } from "./ui/Button.tsx";
import { useT } from "../i18n/catalog.ts";
import { shell } from "../i18n/catalogs/shell.ts";

/**
 * Wraps the page. While the session cannot be checked, the page gives way
 * to a card that keeps retrying by itself; a failed server logout shows as
 * a strip above the page.
 */
export function SessionRecoveryNotice({ children }: { children: ReactNode }) {
  const bootstrap = useSessionProblems((state) => state.bootstrap);
  const unknown = useSession((state) => state.status === "unknown");
  if (bootstrap && unknown) return <SessionCheckFailed />;
  return (
    <>
      <LogoutFailed />
      {children}
    </>
  );
}

function useOnline(): boolean {
  const [online, setOnline] = useState(() =>
    typeof navigator === "undefined" ? true : navigator.onLine,
  );
  useEffect(() => {
    const update = () => setOnline(navigator.onLine);
    window.addEventListener("online", update);
    window.addEventListener("offline", update);
    return () => {
      window.removeEventListener("online", update);
      window.removeEventListener("offline", update);
    };
  }, []);
  return online;
}

const secondsUntil = (at: number | null) =>
  at === null ? 0 : Math.max(0, Math.round((at - Date.now()) / 1000));

function SessionCheckFailed() {
  const t = useT(shell);
  const online = useOnline();
  const checking = useSessionProblems((state) => state.checking);
  const [firstWait] = useState(() =>
    secondsUntil(useSessionProblems.getState().retryAt),
  );
  const { seconds, start } = useCooldown(firstWait);
  // Read out once per planned retry; the visible countdown is not.
  const [planned, setPlanned] = useState(firstWait);
  useEffect(
    () =>
      useSessionProblems.subscribe((state, before) => {
        if (state.retryAt === before.retryAt) return;
        const wait = secondsUntil(state.retryAt);
        setPlanned(wait);
        start(wait);
      }),
    [start],
  );
  return (
    <AuthFrame>
      <section className="lr-auth-card" aria-labelledby="session-check-title">
        <h1 id="session-check-title">
          {t(online ? "recovery.noAnswer" : "recovery.offline")}
        </h1>
        <p className="auth-lead">
          {t(online ? "recovery.noAnswerLead" : "recovery.offlineLead")}
        </p>
        <p className="session-recovery__status">
          <span aria-hidden="true">
            {checking
              ? t("recovery.checking")
              : seconds > 0
                ? t("recovery.retryIn", { seconds })
                : null}
          </span>
          <span role="status" className="sr-only">
            {checking
              ? t("recovery.checking")
              : planned > 0
                ? t("recovery.retryInSeconds", { count: planned })
                : null}
          </span>
        </p>
        <div className="auth-actions">
          <Button
            variant="primary"
            size="lg"
            pending={checking}
            pendingLabel={t("recovery.checking")}
            onClick={() => void retrySession()}
          >
            {t("recovery.retryNow")}
          </Button>
        </div>
      </section>
    </AuthFrame>
  );
}

function LogoutFailed() {
  const t = useT(shell);
  const router = useRouter();
  const problem = useSessionProblems((state) => state.logout);
  const [pending, setPending] = useState(false);
  if (!problem) return null;
  const retry = async () => {
    setPending(true);
    try {
      await logout();
      await router.invalidate();
    } finally {
      setPending(false);
    }
  };
  return (
    <section role="alert" className="session-logout-failed">
      <p>{problem}</p>
      <Button
        size="sm"
        pending={pending}
        pendingLabel={t("recovery.signingOut")}
        onClick={() => void retry()}
      >
        {t("recovery.signOutAgain")}
      </Button>
    </section>
  );
}
