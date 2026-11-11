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

/**
 * Dev only: /public-pages.json as the web container would write it, from
 * the file GELABBER_DEV_PUBLIC_PAGES names; 404 without it, as in an image
 * whose start script never ran. Vite would otherwise answer the app shell.
 */
function devPublicPages(file: string | undefined): Plugin {
  return {
    name: "gelabber-dev-public-pages",
    apply: "serve",
    configureServer(server) {
      server.middlewares.use("/public-pages.json", (_req, res) => {
        const send = (status: number, body: string) => {
          res.statusCode = status;
          res.setHeader("Cache-Control", "no-cache");
          if (status === 200)
            res.setHeader("Content-Type", "application/json; charset=utf-8");
          res.end(body);
        };
        if (!file) return send(404, "");
        // Read afresh on every request (the query), relative to web/.
        const url = new URL(file, `file://${server.config.root}/`);
        url.search = `?t=${Date.now()}`;
        import(/* @vite-ignore */ url.href, { with: { type: "json" } }).then(
          (json: { default: unknown }) =>
            send(200, JSON.stringify(json.default)),
          (error: unknown) => {
            server.config.logger.warn(
              `GELABBER_DEV_PUBLIC_PAGES: cannot read ${file}: ${String(error)}`,
            );
            send(404, "");
          },
        );
      });
    },
  };
}

export default defineConfig(({ mode }) => {
  const env = loadEnv(mode, ".", "GELABBER_");
  const apiTarget = env.GELABBER_API_TARGET ?? "http://127.0.0.1:8080";
  const mediaTarget = env.GELABBER_MEDIA_TARGET ?? "http://127.0.0.1:8081";
  return {
    plugins: [
      themeDefaultsHtml(),
      react(),
      tailwindcss(),
      devPublicPages(env.GELABBER_DEV_PUBLIC_PAGES),
    ],
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
