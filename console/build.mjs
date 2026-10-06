// Bundles the console into dist/: one script, the stylesheet, the page and
// ghostty-web's WebAssembly. With --watch, rebuilds on every change, unminified
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
  copied("node_modules/ghostty-web/ghostty-vt.wasm", "ghostty-vt.wasm"),
];

const options = {
  entryPoints: [{ in: "src/app.ts", out: "app" }, ...staticEntries],
  loader: Object.fromEntries(staticEntries.map((entry) => [extname(entry.in), "copy"])),
  bundle: true,
  format: "esm",
  target: "es2022",
  minify: !watching,
  sourcemap: watching,
  outdir: "dist",
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
