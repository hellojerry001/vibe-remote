import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// 1420 被 design-qa-desktop 的 Vite strictPort 占用，这里用 2410 避免冲突
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: {
    port: 2410,
    strictPort: true,
    host: "127.0.0.1",
  },
  envPrefix: ["VITE_", "TAURI_"],
  build: {
    target: "es2021",
    minify: "esbuild",
    sourcemap: false,
  },
});
