import "./theme/startup.ts";
import { QueryClientProvider } from "@tanstack/react-query";
import { RouterProvider } from "@tanstack/react-router";
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import "./index.css";
import { ensureSession } from "./auth/session.ts";
import { queryClient } from "./queryClient.ts";
import { router } from "./routes.tsx";
import { ThemeController } from "./theme/ThemeController.tsx";
import { InterfacePreferences } from "./interface/InterfacePreferences.tsx";

// Kick off the session bootstrap in parallel with the first render; the
// route guards await the same promise instead of starting a second request.
void ensureSession();

// A file dropped outside a drop target would replace the app with the file.
for (const type of ["dragover", "drop"] as const) {
  window.addEventListener(type, (event) => {
    if (event.dataTransfer?.types.includes("Files")) event.preventDefault();
  });
}

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
