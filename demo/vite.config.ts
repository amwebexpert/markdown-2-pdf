import { defineConfig } from "vite";

export default defineConfig({
  server: {
    // wasm/pkg lives outside this project root (a sibling workspace member).
    fs: { allow: [".."] },
  },
});
