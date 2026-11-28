import { beforeEach, describe, expect, it, vi } from "vitest";
import { page } from "vitest/browser";
import { render } from "vitest-browser-react";

// GEL-423: creating the first server from the welcome page. The new server
// enters the server list before the create form's own success handler runs,
// which navigates and opens the invite dialog; a redirect at that moment
// unmounted the form, and the handler never ran.
const fixture = vi.hoisted(() => ({
  servers: [] as { id: string }[],
}));
vi.mock("../servers/queries.ts", () => ({
  useServers: () => ({
    data: fixture.servers,
    isPending: false,
    isError: false,
    isFetching: false,
    refetch: () => Promise.resolve(),
  }),
}));
vi.mock("../auth/scope.ts", () => ({ useUserId: () => "u1" }));
vi.mock("../auth/session.ts", () => ({
  useSession: (select: (state: unknown) => unknown) =>
    select({ user: { id: "u1", name: "Rafi", avatar_url: "a.png" } }),
}));
vi.mock("../dms/queries.ts", () => ({
  useDms: () => ({ data: [], isPending: false, isError: false }),
}));
vi.mock("../pwa/install.ts", () => ({
  useInstallation: () => false,
  installHintUseful: () => false,
}));
vi.mock("../components/Redirect.tsx", () => ({
  Redirect: ({ params }: { params: { serverId: string } }) => (
    <p>Weiter zu {params.serverId}</p>
  ),
}));
// The dialog as the page sees it: open or not, and closed by its form.
vi.mock("../components/ServerDialogs.tsx", () => ({
  CreateServerDialog: ({
    open,
    onClose,
  }: {
    open: boolean;
    onClose: () => void;
  }) =>
    open ? (
      <section aria-label="Server erstellen">
        <button type="button" onClick={onClose}>
          Fertig
        </button>
      </section>
    ) : null,
  JoinServerForm: () => <form aria-label="Einladung" />,
}));

import { WorkspaceIndexPage } from "./WorkspaceIndexPage.tsx";

beforeEach(() => {
  fixture.servers = [];
});

describe("the welcome page while a server is created", () => {
  it("keeps the create dialog when the new server enters the list, then goes there", async () => {
    const screen = await render(<WorkspaceIndexPage />);
    await screen.getByRole("button", { name: "Server erstellen" }).click();
    const dialog = page.getByRole("region", { name: "Server erstellen" });
    await expect.element(dialog).toBeVisible();
    // The create request answered: the list holds the new server.
    fixture.servers = [{ id: "s-new" }];
    await screen.rerender(<WorkspaceIndexPage />);
    await expect.element(dialog).toBeVisible();
    await expect
      .element(page.getByText("Weiter zu s-new"))
      .not.toBeInTheDocument();
    // Its form is done (navigates, opens the invite) and closes the dialog.
    await dialog.getByRole("button", { name: "Fertig" }).click();
    await expect.element(page.getByText("Weiter zu s-new")).toBeVisible();
  });

  it("goes to the first server at once when nothing is being created", async () => {
    fixture.servers = [{ id: "s-1" }];
    await render(<WorkspaceIndexPage />);
    await expect.element(page.getByText("Weiter zu s-1")).toBeVisible();
  });
});
