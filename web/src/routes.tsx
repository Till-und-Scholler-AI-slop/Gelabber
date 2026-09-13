import { createRootRoute, createRouter } from "@tanstack/react-router";
import { Home } from "./Home.tsx";

const rootRoute = createRootRoute({
  component: Home,
});

export const router = createRouter({ routeTree: rootRoute });

declare module "@tanstack/react-router" {
  interface Register {
    router: typeof router;
  }
}
