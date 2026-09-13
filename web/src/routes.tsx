import {
  Outlet,
  createRootRoute,
  createRoute,
  createRouter,
} from "@tanstack/react-router";
import { useAppStore } from "./store.ts";

function Shell() {
  return <Outlet />;
}

function Home() {
  const name = useAppStore((state) => state.name);
  const apiBase = import.meta.env.VITE_API_BASE_URL ?? "/api";

  return (
    <main className="mx-auto flex min-h-dvh max-w-xl flex-col justify-center gap-4 px-6">
      <h1 className="text-3xl font-semibold tracking-tight">{name}</h1>
      <p className="text-neutral-600">
        Monorepo-Schnitt steht. Chat, Voice und Live kommen in späteren
        Tickets — ohne LiveKit, ohne fremdes Produkt-SDK.
      </p>
      <p className="text-sm text-neutral-500">API: {apiBase}</p>
    </main>
  );
}

const rootRoute = createRootRoute({
  component: Shell,
});

const indexRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/",
  component: Home,
});

const routeTree = rootRoute.addChildren([indexRoute]);

export const router = createRouter({ routeTree });

declare module "@tanstack/react-router" {
  interface Register {
    router: typeof router;
  }
}
