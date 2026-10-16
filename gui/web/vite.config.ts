import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";
import { fileURLToPath } from "node:url";

// Tauri expects a fixed port; the browser preview (no Tauri) uses the IPC mock.
export default defineConfig({
  plugins: [react(), tailwindcss()],
  resolve: { alias: { "@": fileURLToPath(new URL("./src", import.meta.url)) } },
  clearScreen: false,
  server: { port: 5173, strictPort: true, host: "localhost" },
  build: { outDir: "dist", target: "es2021", emptyOutDir: true },
});
