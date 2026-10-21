import { defineConfig } from "vite";
import vue from "@vitejs/plugin-vue";

// @ts-expect-error process is a nodejs global
const host = process.env.TAURI_DEV_HOST;

// https://vite.dev/config/
export default defineConfig(async () => ({
  plugins: [vue()],

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
      // 3. tell Vite to ignore watching `src-tauri`
      ignored: ["**/src-tauri/**"],
    },
  },
  build: {
    chunkSizeWarningLimit: 650,
    rollupOptions: {
      output: {
        manualChunks(id) {
          if (id.includes("node_modules")) {
            if (id.includes("three") || id.includes("skinview3d")) {
              return "vendor-three";
            }
            if (id.includes("deepslate")) {
              return "vendor-deepslate";
            }
            if (
              id.includes("naive-ui") ||
              id.includes("seemly") ||
              id.includes("vooks") ||
              id.includes("evtd")
            ) {
              return "vendor-naive-ui";
            }
            if (id.includes("marked") || id.includes("dompurify")) {
              return "vendor-markdown";
            }
          }
        },
      },
    },
  },
}));
