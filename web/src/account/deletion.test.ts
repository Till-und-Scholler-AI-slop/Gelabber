import { beforeEach, describe, expect, it, vi } from "vitest";

import { ApiError } from "../api/client.ts";
import { deleteAccount } from "../auth/session.ts";
import type { Server } from "../servers/types.ts";
import { leaveVoice } from "../voice/session.ts";
import {
  deleteAndLeave,
  deletionRefusal,
  onDeletionRefused,
  ownedServers,
  ownedServersOf,
  submitDeletion,
} from "./deletion.ts";

vi.mock("../auth/session.ts", () => ({ deleteAccount: vi.fn() }));
vi.mock("../voice/session.ts", () => ({ leaveVoice: vi.fn() }));

beforeEach(() => {
  vi.mocked(deleteAccount).mockReset();
  vi.mocked(leaveVoice).mockReset();
});

const server = (id: string, role: Server["role"]): Server => ({
  id,
  name: `Server ${id}`,
  owner_id: "owner",
  created_at: "2026-10-01T10:00:00Z",
  role,
  permissions: [],
  member_permissions: [],
});

describe("the servers in the way of a deletion", () => {
  it("are the ones the list says this account owns", () => {
    expect(ownedServers(undefined)).toEqual([]);
    expect(ownedServers([server("a", "owner"), server("b", "member")])).toEqual(
      [{ id: "a", name: "Server a" }],
    );
  });

  it("come from a 409 owns_servers with their member counts", () => {
    const error = new ApiError(
      "owns_servers",
      409,
      "Transfer or delete your servers first.",
      {},
      null,
      {
        error: "owns_servers",
        servers: [
          { id: "a", name: "Wohnzimmer", member_count: 5 },
          { id: "b", name: "Werkstatt" },
          { id: 3, name: "broken" },
          null,
        ],
      },
    );
    expect(ownedServersOf(error)).toEqual([
      { id: "a", name: "Wohnzimmer", member_count: 5 },
      { id: "b", name: "Werkstatt" },
    ]);
  });

  it("are not read from any other error", () => {
    expect(
      ownedServersOf(new ApiError("validation_failed", 422, "Invalid.")),
    ).toBeNull();
    expect(ownedServersOf(new Error("offline"))).toBeNull();
    expect(
      ownedServersOf(new ApiError("owns_servers", 409, "Servers.")),
    ).toEqual([]);
  });
});

describe("deleting the account", () => {
  it("leaves the call and goes to the sign-in page with the notice", async () => {
    const order: string[] = [];
    vi.mocked(deleteAccount).mockImplementation(async () => {
      order.push("deleted");
    });
    vi.mocked(leaveVoice).mockImplementation(() => void order.push("voice"));
    const navigate = vi.fn(() => void order.push("navigate"));

    await deleteAndLeave({ password: "secret" }, navigate);

    expect(deleteAccount).toHaveBeenCalledWith({ password: "secret" });
    expect(navigate).toHaveBeenCalledWith({
      to: "/login",
      search: { reason: "deleted" },
    });
    expect(order).toEqual(["deleted", "voice", "navigate"]);
  });

  it("stays put when the API refuses", async () => {
    vi.mocked(deleteAccount).mockRejectedValue(
      new ApiError("validation_failed", 422, "Invalid.", { password: "wrong" }),
    );
    const navigate = vi.fn();
    await expect(
      deleteAndLeave({ password: "nope" }, navigate),
    ).rejects.toMatchObject({ code: "validation_failed" });
    expect(leaveVoice).not.toHaveBeenCalled();
    expect(navigate).not.toHaveBeenCalled();
  });

  it("turns a refusal into what the dialog shows", () => {
    expect(
      deletionRefusal(
        new ApiError("owns_servers", 409, "Servers.", {}, null, {
          servers: [{ id: "a", name: "Wohnzimmer", member_count: 5 }],
        }),
      ),
    ).toEqual({
      kind: "owned",
      servers: [{ id: "a", name: "Wohnzimmer", member_count: 5 }],
    });
    expect(
      deletionRefusal(new ApiError("reauth_required", 403, "Sign in again.")),
    ).toEqual({ kind: "reauth" });
    expect(
      deletionRefusal(
        new ApiError("validation_failed", 422, "Invalid.", { email: "wrong" }),
      ),
    ).toEqual({ kind: "emailWrong" });
    expect(
      deletionRefusal(
        new ApiError("validation_failed", 422, "Invalid.", {
          email: "required",
        }),
      ),
    ).toEqual({ kind: "other" });
    expect(
      deletionRefusal(
        new ApiError("validation_failed", 422, "Invalid.", {
          password: "wrong",
        }),
      ),
    ).toEqual({ kind: "other" });
  });
});

describe("the dialog Konto löschen", () => {
  /** Records which step of the dialog a refusal took. */
  function steps() {
    const taken: unknown[] = [];
    return {
      taken,
      steps: {
        owned: (servers: unknown) => void taken.push(["owned", servers]),
        reauth: () => void taken.push("reauth"),
        emailWrong: () => void taken.push("emailWrong"),
        other: (error: unknown) => void taken.push(["other", error]),
      },
    };
  }

  it("switches to the servers of a 409 owns_servers", () => {
    const { taken, steps: dialog } = steps();
    onDeletionRefused(
      new ApiError("owns_servers", 409, "Servers.", {}, null, {
        servers: [{ id: "s9", name: "Werkstatt", member_count: 2 }],
      }),
      dialog,
    );
    expect(taken).toEqual([
      ["owned", [{ id: "s9", name: "Werkstatt", member_count: 2 }]],
    ]);
  });

  it("asks for a fresh sign-in after reauth_required", () => {
    const { taken, steps: dialog } = steps();
    onDeletionRefused(
      new ApiError("reauth_required", 403, "Sign in again."),
      dialog,
    );
    expect(taken).toEqual(["reauth"]);
  });

  it("marks a wrong address, and shows anything else as it is", () => {
    const { taken, steps: dialog } = steps();
    onDeletionRefused(
      new ApiError("validation_failed", 422, "Invalid.", { email: "wrong" }),
      dialog,
    );
    const wrong = new ApiError("validation_failed", 422, "Invalid.", {
      password: "wrong",
    });
    onDeletionRefused(wrong, dialog);
    expect(taken).toEqual(["emailWrong", ["other", wrong]]);
  });

  /** Submits `value` and records what happened. */
  function submit(value: string, hasPassword: boolean, pending = false) {
    const happened: unknown[] = [];
    submitDeletion(value, hasPassword, {
      pending,
      required: () => void happened.push("required"),
      send: (confirmation) => void happened.push(confirmation),
    });
    return happened;
  }

  it("requires the field and sends nothing while it is empty", () => {
    expect(submit("", true)).toEqual(["required"]);
    expect(submit("", false)).toEqual(["required"]);
    expect(submit("   ", false)).toEqual(["required"]);
  });

  it("sends a password as typed, spaces included, and an address trimmed", () => {
    expect(submit("  ", true)).toEqual([{ password: "  " }]);
    expect(submit(" secret ", true)).toEqual([{ password: " secret " }]);
    expect(submit(" ada@example.com ", false)).toEqual([
      { email: "ada@example.com" },
    ]);
  });

  it("sends nothing while a deletion is on its way", () => {
    expect(submit("secret", true, true)).toEqual([]);
    expect(submit("", true, true)).toEqual([]);
  });
});
