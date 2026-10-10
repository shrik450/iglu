// Bundles the console into dist/: one script, the stylesheets, the page and
// the terminal's WebAssembly. With --watch, rebuilds on every change, unminified
// and with a source map, for the local dev stack to serve.
import { build, context } from "esbuild";
import { readdirSync, rmSync, statSync } from "node:fs";
import { extname, join } from "node:path";

const watching = process.argv.includes("--watch");

// Static files are entry points that esbuild copies as they are, so its watcher
// covers them too. A file added to public/ needs the watcher restarted.
function copied(file, out) {
  return { in: file, out: out.slice(0, -extname(out).length) };
}
const staticEntries = [
  ...readdirSync("public", { recursive: true })
    .filter((file) => statSync(join("public", file)).isFile())
    .map((file) => copied(join("public", file), file)),
  copied("node_modules/@wterm/ghostty/wasm/ghostty-vt.wasm", "ghostty-vt.wasm"),
  copied("vendor/wterm-dom/src/terminal.css", "wterm.css"),
  // Served by iglud itself: its content security policy allows no other origin.
  ...["unbounded", "geist", "jetbrains-mono"].map((font) =>
    copied(`node_modules/@fontsource-variable/${font}/files/${font}-latin-wght-normal.woff2`, `fonts/${font}.woff2`),
  ),
];

const options = {
  entryPoints: [{ in: "src/main.tsx", out: "app" }, ...staticEntries],
  loader: Object.fromEntries(staticEntries.map((entry) => [extname(entry.in), "copy"])),
  bundle: true,
  jsx: "automatic",
  jsxImportSource: "preact",
  format: "esm",
  target: "es2022",
  minify: !watching,
  sourcemap: watching,
  outdir: "dist",
  // wterm's DOM renderer is vendored: see vendor/wterm-dom/UPSTREAM.
  alias: { "@wterm/dom": "./vendor/wterm-dom/src/index.ts" },
};

rmSync("dist", { recursive: true, force: true });
if (watching) {
  const report = {
    name: "report",
    setup(build) {
      build.onEnd((result) => {
        if (result.errors.length === 0) console.log(`console rebuilt at ${new Date().toLocaleTimeString()}`);
      });
    },
  };
  const ctx = await context({ ...options, plugins: [report] });
  await ctx.watch();
} else {
  await build(options);
}
