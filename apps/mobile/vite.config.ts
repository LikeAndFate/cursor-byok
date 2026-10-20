import { defineConfig } from "vitest/config";
import react from "@vitejs/plugin-react";
import { VitePWA } from "vite-plugin-pwa";

export default defineConfig({
  base: "/mobile/",
  plugins: [
    react(),
    VitePWA({
      registerType: "autoUpdate",
      scope: "/mobile/",
      base: "/mobile/",
      includeAssets: ["icons/icon.svg"],
      manifest: {
        name: "Cursor BYTK 手机远控",
        short_name: "BYTK 远控",
        start_url: "/mobile/#/",
        scope: "/mobile/",
        display: "standalone",
        background_color: "#0b0d12",
        theme_color: "#0b0d12",
        icons: [{ src:"icons/icon.svg", sizes:"any", type:"image/svg+xml", purpose:"any maskable" }],
      },
      workbox: {
        globPatterns: ["**/*.{js,css,html,svg,png,woff2}"],
        navigateFallback: "/mobile/index.html",
        navigateFallbackDenylist: [/^\/api\//],
        runtimeCaching: [],
        cleanupOutdatedCaches: true,
      },
    }),
  ],
  server: {
    host: "127.0.0.1",
    port: 43128,
    strictPort: true,
    proxy: { "/api": { target: "http://127.0.0.1:43127", changeOrigin: false } },
  },
  test: { environment:"jsdom", setupFiles:["./src/test/setup.ts"], globals:true },
});
