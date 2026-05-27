import { defineConfig } from "vite";
import { configDefaults } from "vitest/config";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";

// @ts-expect-error process is a nodejs global
const host = process.env.TAURI_DEV_HOST;

// https://vite.dev/config/
export default defineConfig(async () => ({
  plugins: [react(), tailwindcss()],

  // Vite options tailored for Tauri development and only applied in `tauri dev` or `tauri build`
  //
  // 1. prevent Vite from obscuring rust errors
  clearScreen: false,
  // 2. tauri expects a fixed port, fail if that port is not available
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host
      ? {
          protocol: "ws",
          host,
          port: 1421,
        }
      : undefined,
    watch: {
      // Skip large/irrelevant trees so chokidar doesn't crawl them at
      // startup. The renderer only lives in src/ + public/ + index.html;
      // everything else is C++/Rust/build state.
      ignored: [
        "**/src-tauri/**",     // Rust backend
        "**/external/**",      // OrcaSlicer submodule (~750 MB, tens of thousands of files)
        "**/crates/**",        // slic3r-ffi crate (Rust + C++ shim)
        "**/build/**",         // cmake output
        "**/target/**",        // cargo output
        "**/docs/**",          // design docs
        "**/scripts/**",       // shell helpers
        "**/packaging/**",     // makepkg artifacts (root-only fakeroot mode bits)
      ],
    },
  },
  test: {
    // Extend vitest's default excludes with the packaging tree.
    // makepkg leaves root-owned, mode-0111 directories under
    // packaging/arch/ (fakeroot $pkgdir) which vitest's test-file
    // scanner trips over (EACCES). The whole packaging/ subtree is
    // build output, never test source — safe to skip wholesale.
    exclude: [...configDefaults.exclude, "**/packaging/**"],
  },
}));
