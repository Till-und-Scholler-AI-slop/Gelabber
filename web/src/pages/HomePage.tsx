import { Link } from "@tanstack/react-router";

import { useSession } from "../auth/session.ts";

export function HomePage() {
  const user = useSession((state) => state.user);

  return (
    <section className="flex flex-col gap-4">
      <h1 className="text-3xl font-semibold tracking-tight">
        {user ? `Hallo ${user.name}` : "Hallo"}
      </h1>
      <p className="max-w-prose text-neutral-600">
        Du bist angemeldet. Chat, Voice und Live kommen in den nächsten Tickets
        — ohne LiveKit, ohne fremdes Produkt-SDK.
      </p>
      <p className="text-sm text-neutral-500">
        <Link
          to="/profile"
          className="font-medium text-neutral-900 underline underline-offset-2"
        >
          Profil bearbeiten
        </Link>
      </p>
    </section>
  );
}
