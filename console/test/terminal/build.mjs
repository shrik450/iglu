// Bundles the terminal tier's page into the directory given, default
// .dev/terminal: the console's terminal on a page of its own, with the
// console's stylesheets, fonts and WebAssembly, as build.mjs bundles them.
//
//     node test/terminal/build.mjs [out]
import { build } from "esbuild";
import { readFileSync, rmSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const console_ = resolve(here, "../..");
const out = resolve(process.argv[2] ?? join(console_, "../.dev/terminal"));

rmSync(out, { recursive: true, force: true });
await build({
  absWorkingDir: console_,
  entryPoints: [
    { in: "test/terminal/harness.ts", out: "harness" },
    { in: "src/app.css", out: "app" },
    { in: "vendor/wterm-dom/src/terminal.css", out: "wterm" },
  ],
  assetNames: "assets/[name]-[hash]",
  publicPath: "/",
  loader: { ".woff2": "file", ".wasm": "file" },
  bundle: true,
  format: "esm",
  target: "es2022",
  sourcemap: "linked",
  outdir: out,
  // As build.mjs has it: wterm's DOM renderer is vendored.
  alias: { "@wterm/dom": "./vendor/wterm-dom/src/index.ts" },
});
writeFileSync(join(out, "index.html"), readFileSync(join(here, "index.html")));
