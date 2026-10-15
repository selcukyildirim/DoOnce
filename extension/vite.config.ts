import { resolve } from "node:path";
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import { copyFileSync, cpSync, mkdirSync } from "node:fs";

export default defineConfig({
  // Chrome extension pages cannot load "/assets/..." — paths must be relative.
  base: "./",
  plugins: [
    react(),
    {
      name: "copy-manifest",
      closeBundle() {
        const out = resolve(__dirname, "dist");
        mkdirSync(out, { recursive: true });
        copyFileSync(resolve(__dirname, "manifest.json"), resolve(out, "manifest.json"));
        cpSync(resolve(__dirname, "icons"), resolve(out, "icons"), { recursive: true });
      },
    },
  ],
  build: {
    outDir: "dist",
    emptyOutDir: false,
    rollupOptions: {
      input: {
        sidepanel: resolve(__dirname, "sidepanel.html"),
        background: resolve(__dirname, "src/background/index.ts"),
      },
      output: {
        entryFileNames: (chunk) => (chunk.name === "background" ? "background.js" : "assets/[name].js"),
        chunkFileNames: "assets/[name]-[hash].js",
        assetFileNames: "assets/[name][extname]",
      },
    },
  },
});
