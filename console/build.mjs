// Bundles the console into dist/: one script, the stylesheet, the page and
// ghostty-web's WebAssembly.
import { build } from "esbuild";
import { cpSync, mkdirSync } from "node:fs";

mkdirSync("dist", { recursive: true });
await build({
  entryPoints: ["src/app.ts"],
  bundle: true,
  format: "esm",
  target: "es2022",
  minify: true,
  sourcemap: false,
  outfile: "dist/app.js",
});
cpSync("public", "dist", { recursive: true });
cpSync("node_modules/ghostty-web/ghostty-vt.wasm", "dist/ghostty-vt.wasm");
