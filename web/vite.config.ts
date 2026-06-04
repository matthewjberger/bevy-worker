import { defineConfig } from "vite";

// On GitHub Pages the app is served from /bevy-worker/, but a plain dev server
// serves from /. Only apply the base path for production builds.
export default defineConfig(({ command }) => ({
  base: command === "build" ? "/bevy-worker/" : "/",
  worker: {
    format: "es",
  },
}));
