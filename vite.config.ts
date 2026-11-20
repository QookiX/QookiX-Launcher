import { defineConfig } from "vite";
import vue from "@vitejs/plugin-vue";
import { changelogPlugin } from "./scripts/shared/vite-changelog.mjs";

// @ts-expect-error process is a nodejs global
const host = process.env.TAURI_DEV_HOST;

// https://vite.dev/config/
export default defineConfig(async () => ({
  plugins: [vue(), changelogPlugin()],

  // 生产构建：只去掉 debugger，保留 console 以便用户日志排查
  esbuild: {
    drop: ["debugger"],
    legalComments: "none",
  },

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
    // 体积优化：JS 用 esbuild 压缩、CSS 单独压缩、去掉 debugger、不保留 license 注释块。
    // 不 drop console —— 启动器出问题时用户日志里需要它。
    minify: "esbuild",
    cssMinify: "esbuild",
    reportCompressedSize: true,
    chunkSizeWarningLimit: 650,
    rollupOptions: {
      output: {
        manualChunks(id) {
          const dependency = id.replaceAll("\\", "/");
          if (dependency.includes("/node_modules/skinview3d/node_modules/three/")) {
            return "vendor-skin-three";
          }
          if (dependency.includes("/node_modules/skinview3d/")) {
            return "vendor-skin-viewer";
          }
          if (dependency.endsWith("/node_modules/three/build/three.core.js")) {
            return "vendor-three-core";
          }
          if (dependency.includes("/node_modules/three/")) {
            return "vendor-three-renderer";
          }
          if (dependency.includes("/node_modules/deepslate/")) {
            return "vendor-deepslate";
          }
          if (/\/node_modules\/(?:vue|vue-router|pinia|vue-i18n|@vue\/[^/]+|@intlify\/[^/]+|vue-demi)\//.test(dependency)) {
            return "vendor-vue";
          }
          if (dependency.includes("/node_modules/lodash-es/")) {
            return "vendor-lodash";
          }
          if (/\/node_modules\/(?:naive-ui|seemly|vooks|evtd|vueuc|css-render|@css-render\/[^/]+|date-fns|treemate|async-validator)\//.test(dependency)) {
            return "vendor-naive-core";
          }
          if (/\/node_modules\/(marked|dompurify)\//.test(dependency)) {
            return "vendor-markdown";
          }
        },
      },
    },
  },
}));
