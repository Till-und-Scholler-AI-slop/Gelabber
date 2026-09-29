/* global URL */
import { api, check, click, navigate, observe, until } from "./harness.mjs";

async function login(actor, email, password) {
  await actor.page.getByLabel("E-Mail-Adresse").fill(email);
  await actor.page.getByLabel("Passwort", { exact: true }).fill(password);
  await click(actor, "Anmelden");
  await actor.page.waitForURL((url) => !url.pathname.includes("login"));
}
export async function accountSwitchScenarios(h, f) {
  for (const status of [401, 403]) {
    await h.run(
      `two-tabs-account-switch-delayed-${status}`,
      ["04"],
      async () => {
        await navigate(f.owner, "/profile", f.base);
        const monitor = await f.owner.context.newPage();
        await monitor.goto(`${f.base}/profile`);
        let release,
          requests = 0;
        const gate = new Promise((resolve) => {
          release = resolve;
        });
        const endpoint = "**/api/me";
        await f.owner.page.route(endpoint, async (route) => {
          if (route.request().method() !== "PATCH") return route.continue();
          requests++;
          if (requests === 1) {
            await gate;
            await route.fulfill({
              status,
              contentType: "application/json",
              body:
                status === 403
                  ? '{"error":"csrf_failed","message":"E2E synthetic stale CSRF"}'
                  : '{"error":"unauthorized","message":"E2E synthetic delayed 401"}',
            });
          } else await route.continue();
        });
        try {
          await f.owner.page
            .getByLabel("Name", { exact: true })
            .fill("E2E delayed A mutation");
          await click(f.owner, "Speichern");
          await until(
            async () => ({ requests }),
            (r) => r.requests === 1,
            "old-account-patch-not-held",
          );
          // Same-document logout/login updates the app generation while A's request remains in flight.
          const logoutResponse = f.owner.page.waitForResponse(
            (r) =>
              new URL(r.url()).pathname === "/api/auth/logout" &&
              r.request().method() === "POST",
          );
          await click(f.owner, "Abmelden");
          await logoutResponse;
          await f.owner.page.waitForURL(/\/login/);
          await login(f.owner, f.member.email, f.member.password);
          await f.owner.page
            .getByRole("link", { name: "E2E Member", exact: true })
            .click();
          const before = await api({ page: monitor }, "/auth/session");
          check(
            before.body.user?.id === f.member.id,
            "second-tab-cookie-not-account-b",
          );
          release();
          await observe(1_000, async () => ({ requests }));
          const after = await api({ page: monitor }, "/auth/session");
          const name = await f.owner.page
            .getByLabel("Name", { exact: true })
            .inputValue();
          check(
            requests === 1 &&
              after.body.user?.id === f.member.id &&
              after.body.user.name === "E2E Member" &&
              name === "E2E Member",
            "late-a-response-mutated-or-logged-out-b",
            {
              oldPatchRequests: requests,
              sessionStillB: after.body.user?.id === f.member.id,
              profileStillB: name === "E2E Member",
            },
          );
          return {
            delayedStatus: status,
            oldPatchRequests: requests,
            secondTabSessionB: true,
            profileBUnchanged: true,
          };
        } finally {
          release();
          await f.owner.page.unroute(endpoint);
          await monitor.close();
          // Restore the fixture's original owner via the real UI, even if the assertion was red.
          const loggedIn = await f.owner.page
            .getByRole("button", { name: "Abmelden", exact: true })
            .count();
          if (loggedIn) {
            const response = f.owner.page.waitForResponse(
              (r) =>
                new URL(r.url()).pathname === "/api/auth/logout" &&
                r.request().method() === "POST",
            );
            await click(f.owner, "Abmelden");
            await response;
          }
          await f.owner.page.waitForURL(/\/login/);
          await login(f.owner, f.owner.email, f.owner.password);
        }
      },
    );
  }
}
