import react from "@vitejs/plugin-react"
import { fileURLToPath } from "node:url"
import { defineConfig } from "vite"

// Deployed as a GitHub Pages project site at /ucabled/, so production URLs
// are absolute; the dev server stays at the root for short local URLs.
const PROD_BASE = "/ucabled/"

// https://vite.dev/config/
export default defineConfig(({ command, isPreview, isSsrBuild }) => ({
  base: command === "build" || isPreview ? PROD_BASE : "/",
  plugins: [react()],
  server: {
    port: 3000,
  },
  build: isSsrBuild
    ? {
        // consumed by scripts/prerender.mjs, then deleted
        outDir: "dist-ssr",
      }
    : {
        // two static pages: English at / and Chinese at /zh/
        rollupOptions: {
          input: {
            main: fileURLToPath(new URL("index.html", import.meta.url)),
            zh: fileURLToPath(new URL("zh/index.html", import.meta.url)),
          },
        },
      },
}))
