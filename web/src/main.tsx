import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { RouterProvider } from "@tanstack/react-router";
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import "./index.css";
import { ensureSession } from "./auth/session.ts";
import { router } from "./routes.tsx";
import { ThemeController } from "./theme/ThemeController.tsx";

const queryClient = new QueryClient({
  defaultOptions: {
    mutations: { retry: false },
  },
});

// Kick off the session bootstrap in parallel with the first render; the
// route guards await the same promise instead of starting a second request.
void ensureSession();

const root = document.getElementById("root");
if (!root) {
  throw new Error("root element missing");
}

createRoot(root).render(
  <StrictMode>
    <ThemeController />
    <QueryClientProvider client={queryClient}>
      <RouterProvider router={router} />
    </QueryClientProvider>
  </StrictMode>,
);
