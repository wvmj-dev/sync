import { defineConfig } from "vite";
import vue from "@vitejs/plugin-vue";
import process from "node:process";

const host = process.env.TAURI_DEV_HOST;

// O frontend vive em ui/; o build sai em dist/ (frontendDist do Tauri).
export default defineConfig({
  root: "ui",
  plugins: [vue()],
  build: {
    outDir: "../dist",
    emptyOutDir: true,
    target: "es2022",
  },
  // Não esconder erros do Rust no terminal.
  clearScreen: false,
  server: {
    // O Tauri espera uma porta fixa.
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host ? { protocol: "ws", host, port: 1421 } : undefined,
    watch: {
      ignored: ["**/src-tauri/**", "**/crates/**", "**/target/**"],
    },
  },
});
