import { api, check, until } from "./harness.mjs";

export async function membershipFaultScenarios(h, f, runtime) {
  const id = "ban-invite-join-race";
  if (!runtime) {
    h.blocked(
      id,
      "requires own API database and a fixture-owned server row lock to prove concurrent transactions",
      ["03a"],
    );
    return;
  }
  await h.run(id, ["03a"], async () => {
    const controls = [];
    for (const first of ["ban", "join"]) {
      const candidate = await h.actor(`RaceCandidate${first}`);
      const invite = await api(
        f.owner,
        `/servers/${f.serverId}/invites`,
        "POST",
        {},
      );
      check(invite.status === 201, "fixture-race-invite-not-created");
      const path = `/invites/${invite.body.code}/join`;
      check(
        (await api(candidate, `/invites/${invite.body.code}`)).status === 200,
        "fixture-race-invite-preview-failed",
      );
      const lock = await runtime.lockOwnedServer(f.serverId, f.owner.id);
      const requests = [];
      try {
        const launch = (kind) => {
          const pending =
            kind === "ban"
              ? api(f.owner, `/servers/${f.serverId}/ban`, "POST", {
                  user_id: candidate.id,
                })
              : api(candidate, path, "POST");
          // Handlers settle even if an early lock assertion fails.
          pending.catch(() => {});
          requests.push({ kind, pending });
        };
        launch(first);
        await until(
          lock.waiting,
          (n) => n === 1,
          "fixture-first-race-transaction-not-blocked",
        );
        launch(first === "ban" ? "join" : "ban");
        await until(
          lock.waiting,
          (n) => n === 2,
          "fixture-two-race-transactions-not-blocked",
        );
        await lock.release();
        const results = Object.fromEntries(
          await Promise.all(
            requests.map(async (r) => [r.kind, (await r.pending).status]),
          ),
        );
        check(
          results.ban === 204 &&
            (first === "ban"
              ? results.join === 403
              : [200, 404].includes(results.join)),
          "concurrent-ban-join-status-invalid",
          { controlConfirmed: true, first, statuses: results },
        );
        const memberRows = Number(
          await runtime.sql(
            `SELECT count(*) FROM server_members WHERE server_id='${f.serverId}'::uuid AND user_id='${candidate.id}'::uuid`,
          ),
        );
        const banRows = Number(
          await runtime.sql(
            `SELECT count(*) FROM server_bans WHERE server_id='${f.serverId}'::uuid AND user_id='${candidate.id}'::uuid`,
          ),
        );
        const laterJoin = await api(candidate, path, "POST");
        check(
          memberRows === 0 && banRows === 1 && laterJoin.status === 403,
          "ban-race-left-membership-or-allowed-rejoin",
          {
            controlConfirmed: true,
            first,
            memberRows,
            banRows,
            laterJoinStatus: laterJoin.status,
          },
        );
        controls.push({
          first,
          twoBlockedTransactionsObserved: true,
          ...results,
          finalMemberRows: memberRows,
          finalBanRows: banRows,
          subsequentJoinStatus: laterJoin.status,
        });
      } finally {
        await lock.release();
        await Promise.allSettled(requests.map((r) => r.pending));
        await candidate.context.close();
      }
    }
    return { fixtureOwnedServerLock: true, controls };
  });
}
