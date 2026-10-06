import { useRouter } from "@tanstack/react-router";
import { useState } from "react";
import { ensureSession, logout, useSessionProblems } from "../auth/session.ts";

export function SessionRecoveryNotice() {
  const router = useRouter();
  const problems = useSessionProblems();
  const [pending, setPending] = useState(false);
  if (!problems.bootstrap && !problems.logout) return null;
  const retry = async () => {
    setPending(true);
    try {
      if (problems.logout) await logout();
      else await ensureSession();
      await router.invalidate();
    } finally {
      setPending(false);
    }
  };
  return (
    <section
      role="alert"
      className="mx-4 my-3 rounded-lg border border-amber-400 bg-amber-50 p-4 text-sm text-neutral-900 dark:bg-neutral-900 dark:text-neutral-100"
    >
      <p>{problems.logout ?? problems.bootstrap}</p>
      <button
        type="button"
        disabled={pending}
        onClick={() => void retry()}
        className="mt-2 rounded border border-neutral-400 px-3 py-2 font-medium disabled:opacity-50"
      >
        {problems.logout ? "Erneut abmelden" : "Sitzung erneut prüfen"}
      </button>
    </section>
  );
}
