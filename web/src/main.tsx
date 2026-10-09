import "./theme/startup.ts";
import { QueryClientProvider } from "@tanstack/react-query";
import { RouterProvider } from "@tanstack/react-router";
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import "./index.css";
import { ensureSession, useSession } from "./auth/session.ts";
import { queryClient } from "./queryClient.ts";
import { router } from "./routes.tsx";
import { ThemeController } from "./theme/ThemeController.tsx";
import { InterfacePreferences } from "./interface/InterfacePreferences.tsx";
import { isDesktopApp } from "./voice/native/bridge.ts";
import { trackInstallation } from "./pwa/install.ts";
import { startPwa } from "./pwa/start.ts";

const desktop = isDesktopApp();
const stopInstallTracking = trackInstallation(window, desktop);
if (import.meta.hot) import.meta.hot.dispose(stopInstallTracking);
startPwa({
  desktop,
  user: () => useSession.getState().user?.id,
  navigate: (to) => router.navigate(to),
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
    <InterfacePreferences />
    <QueryClientProvider client={queryClient}>
      <RouterProvider router={router} />
    </QueryClientProvider>
  </StrictMode>,
);
