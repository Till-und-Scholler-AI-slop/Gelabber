import tailwindcss from "@tailwindcss/vite";
import react from "@vitejs/plugin-react";
import { defineConfig, loadEnv, type Plugin } from "vite";

import { fillThemeColours } from "./src/theme/themeHtml.ts";

function themeDefaultsHtml(): Plugin {
  return {
    name: "gelabber-theme-defaults",
    transformIndexHtml: { order: "pre", handler: fillThemeColours },
  };
}

export default defineConfig(({ mode }) => {
  const env = loadEnv(mode, ".", "GELABBER_");
  const apiTarget = env.GELABBER_API_TARGET ?? "http://127.0.0.1:8080";
  const mediaTarget = env.GELABBER_MEDIA_TARGET ?? "http://127.0.0.1:8081";
  return {
    plugins: [themeDefaultsHtml(), react(), tailwindcss()],
    server: {
      // Same-origin `/api` in dev too, so the httpOnly session cookie works
      // without CORS. Behind Compose, Caddy does the same job.
      proxy: {
        "/api": {
          target: apiTarget,
          changeOrigin: false,
        },
        "/ws": {
          target: apiTarget,
          changeOrigin: false,
          ws: true,
        },
        "/media": {
          target: mediaTarget,
          changeOrigin: false,
          ws: true,
        },
      },
    },
  };
});
