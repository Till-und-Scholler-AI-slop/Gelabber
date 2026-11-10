import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { InviteTerms } from "../servers/inviteTerms.ts";
import type { Invite } from "../servers/types.ts";
import { INVITE_TERMS_DELAY_MS, inviteSession } from "./inviteSession.ts";

const WEEK: InviteTerms = { expiry: "7d", uses: "unlimited" };
const HOUR: InviteTerms = { expiry: "1h", uses: "unlimited" };
const DAY: InviteTerms = { expiry: "24h", uses: "unlimited" };

function setup() {
  let n = 0;
  const answers: {
    resolve: (invite: Invite) => void;
    reject: (error: Error) => void;
  }[] = [];
  const create = vi.fn(
    () =>
      new Promise<Invite>((resolve, reject) =>
        answers.push({ resolve, reject }),
      ),
  );
  const invite = () => ({ code: `code-${++n}` }) as Invite;
  const revoke = vi.fn();
  const shown = vi.fn();
  const failed = vi.fn();
  const session = inviteSession({ create, revoke, shown, failed });
  return { session, create, answers, invite, revoke, shown, failed };
}

describe("an invite dialog's links", () => {
  beforeEach(() =>
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] }),
  );
  afterEach(() => vi.useRealTimers());

  it("revokes a link nobody copied when the dialog closes", async () => {
    const { session, answers, invite, revoke, shown } = setup();
    session.request(WEEK, false);
    const first = invite();
    answers[0]!.resolve(first);
    await vi.waitFor(() => expect(shown).toHaveBeenCalledOnce());
    expect(revoke).not.toHaveBeenCalled();
    session.close();
    expect(revoke).toHaveBeenCalledWith(first.code);
  });

  it("keeps a link that was copied or shared", async () => {
    const { session, answers, invite, revoke, shown } = setup();
    session.request(WEEK, false);
    answers[0]!.resolve(invite());
    await vi.waitFor(() => expect(shown).toHaveBeenCalled());
    session.handOut();
    session.close();
    expect(revoke).not.toHaveBeenCalled();
  });

  it("revokes a link that arrives after the dialog closed", async () => {
    const { session, answers, invite, revoke, shown } = setup();
    session.request(WEEK, false);
    session.close();
    const late = invite();
    answers[0]!.resolve(late);
    await vi.waitFor(() => expect(revoke).toHaveBeenCalledWith(late.code));
    expect(shown).not.toHaveBeenCalled();
  });

  it("makes one link for a run of chip changes", async () => {
    const { session, create, answers, invite, revoke, shown } = setup();
    session.request(WEEK, false);
    const first = invite();
    answers[0]!.resolve(first);
    await vi.waitFor(() => expect(shown).toHaveBeenCalledOnce());
    session.request(DAY, true);
    session.request(HOUR, true);
    expect(create).toHaveBeenCalledOnce();
    vi.advanceTimersByTime(INVITE_TERMS_DELAY_MS);
    expect(create).toHaveBeenCalledTimes(2);
    expect(create).toHaveBeenLastCalledWith(HOUR);
    const second = invite();
    answers[1]!.resolve(second);
    await vi.waitFor(() => expect(shown).toHaveBeenCalledTimes(2));
    expect(shown).toHaveBeenLastCalledWith(second, HOUR, true);
    expect(revoke).toHaveBeenCalledExactlyOnceWith(first.code);
  });

  it("drops a change still waiting when the dialog closes", () => {
    const { session, create } = setup();
    session.request(DAY, true);
    session.close();
    vi.advanceTimersByTime(INVITE_TERMS_DELAY_MS);
    expect(create).not.toHaveBeenCalled();
  });

  it("revokes a link overtaken by newer terms, and hands back the old terms on failure", async () => {
    const { session, answers, invite, revoke, shown, failed } = setup();
    session.request(WEEK, false);
    answers[0]!.resolve(invite());
    await vi.waitFor(() => expect(shown).toHaveBeenCalledOnce());
    session.request(DAY, false);
    session.request(HOUR, false);
    const overtaken = invite();
    answers[1]!.resolve(overtaken);
    await vi.waitFor(() => expect(revoke).toHaveBeenCalledWith(overtaken.code));
    answers[2]!.reject(new Error("nope"));
    await vi.waitFor(() => expect(failed).toHaveBeenCalledWith(WEEK));
    expect(shown).toHaveBeenCalledOnce();
  });

  it("opens again after StrictMode's trial unmount", async () => {
    const { session, answers, invite, revoke, shown } = setup();
    session.request(WEEK, false);
    session.close();
    session.open();
    answers[0]!.resolve(invite());
    await vi.waitFor(() => expect(shown).toHaveBeenCalledOnce());
    expect(revoke).not.toHaveBeenCalled();
  });

  it("makes no new link when the chips come back to the shown terms", async () => {
    const { session, create, answers, invite, revoke, shown } = setup();
    session.request(WEEK, false);
    answers[0]!.resolve(invite());
    await vi.waitFor(() => expect(shown).toHaveBeenCalledOnce());
    session.request(DAY, true);
    session.request({ ...WEEK }, true);
    vi.advanceTimersByTime(INVITE_TERMS_DELAY_MS);
    expect(create).toHaveBeenCalledOnce();
    expect(revoke).not.toHaveBeenCalled();
  });

  it("drops a link on its way when the chips come back to the shown terms", async () => {
    const { session, create, answers, invite, revoke, shown } = setup();
    session.request(WEEK, false);
    answers[0]!.resolve(invite());
    await vi.waitFor(() => expect(shown).toHaveBeenCalledOnce());
    session.request(DAY, false);
    session.request(WEEK, true);
    vi.advanceTimersByTime(INVITE_TERMS_DELAY_MS);
    expect(create).toHaveBeenCalledTimes(2);
    const unwanted = invite();
    answers[1]!.resolve(unwanted);
    await vi.waitFor(() => expect(revoke).toHaveBeenCalledWith(unwanted.code));
    expect(shown).toHaveBeenCalledOnce();
  });
});
