// `vitest/config` re-exports Vite's defineConfig with the `test` key typed (RESEARCH Q8).
import { defineConfig } from "vitest/config";

export default defineConfig({
  // Tauri `devUrl` (Plan 01-03) points at this fixed port.
  server: { port: 1420, strictPort: true },
  build: {
    rollupOptions: {
      input: {
        overlay: "overlay.html",
        options: "options.html",
        about: "about.html",
        permission: "permission.html",
        print: "print.html",
      },
    },
  },
  test: {
    environment: "node",
    // Both globs stay inside the Vite root. `test/**` hosts Plan 01-05's harness
    // files and Plan 01-07's `ui/test/spike-stats.test.ts`.
    include: ["src/**/*.test.ts", "test/**/*.test.ts"],
    // Vitest 5's default reporter prints only a summary in a non-TTY (CI, `bun run`);
    // `tree` prints one line per test file so CI logs and the purity/glob checks can
    // see which files ran.
    reporters: ["tree"],
  },
});
