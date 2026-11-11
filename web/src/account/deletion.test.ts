import { beforeEach, describe, expect, it, vi } from "vitest";

import { ApiError } from "../api/client.ts";
import { deleteAccount } from "../auth/session.ts";
import type { Server } from "../servers/types.ts";
import { leaveVoice } from "../voice/session.ts";
import {
  deleteAndLeave,
  deletionRefusal,
  ownedServers,
  ownedServersOf,
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
