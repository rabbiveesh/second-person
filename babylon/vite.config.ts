import { defineConfig } from "vitest/config";

export default defineConfig({
  // Relative URLs so the build works under /<repo>/<subdir>/ on Pages.
  base: "./",
  // Share the Bevy game's procedurally generated SFX instead of copying them.
  publicDir: "../assets",
  build: { target: "es2022", chunkSizeWarningLimit: 8000 },
  optimizeDeps: { exclude: ["@babylonjs/havok", "recast-navigation"] },
  test: { include: ["tests/**/*.test.ts"], testTimeout: 60_000 },
});
