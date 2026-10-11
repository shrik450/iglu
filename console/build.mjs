// Bundles the console into dist/: its page, and under assets/ the script, the
// stylesheets, the fonts and the terminal's WebAssembly, each named for what's
// in it. iglud lets browsers keep assets for good and asks them to check the
// page every time, so a deploy reaches every tab on its next load. assets.json
// names the stylesheet for iglud's own pages. With --watch, rebuilds on every
// change, unminified and with a source map, for the local dev stack to serve;
// a change to the page itself needs the watcher restarted.
import { build, context } from "esbuild";
import { readdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { basename, join, relative } from "node:path";

const watching = process.argv.includes("--watch");

// What the page and iglud ask for, and the entry point that makes it.
const entries = {
  "app.js": "src/main.tsx",
  "app.css": "src/app.css",
  "wterm.css": "vendor/wterm-dom/src/terminal.css",
};

const options = {
  entryPoints: Object.entries(entries).map(([name, file]) => ({ in: file, out: name.replace(/\.\w+$/, "") })),
  entryNames: "assets/[name]-[hash]",
  assetNames: "assets/[name]-[hash]",
  publicPath: "/",
  loader: { ".woff2": "file", ".wasm": "file" },
  metafile: true,
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

// Writes the page and assets.json for a finished build, and removes assets an
// earlier build in this watch left behind.
function finish(metafile) {
  const paths = {};
  for (const [output, { entryPoint }] of Object.entries(metafile.outputs)) {
    const name = Object.keys(entries).find((key) => entries[key] === entryPoint && output.endsWith(key.slice(key.lastIndexOf("."))));
    if (name) paths[name] = `/${relative("dist", output)}`;
  }
  const page = readFileSync("src/index.html", "utf8").replace(/\{\{([\w.]+)\}\}/g, (_, name) => {
    if (!paths[name]) throw new Error(`index.html asks for ${name}, which the build doesn't make`);
    return paths[name];
  });
  writeFileSync("dist/index.html", page);
  writeFileSync("dist/assets.json", `${JSON.stringify(paths, null, 2)}\n`);
  const made = new Set(Object.keys(metafile.outputs).map((output) => basename(output)));
  for (const file of readdirSync("dist/assets")) {
    if (!made.has(file)) rmSync(join("dist/assets", file));
  }
}

rmSync("dist", { recursive: true, force: true });
if (watching) {
  const report = {
    name: "report",
    setup(build) {
      build.onEnd((result) => {
        if (result.errors.length > 0) return;
        finish(result.metafile);
        console.log(`console rebuilt at ${new Date().toLocaleTimeString()}`);
      });
    },
  };
  const ctx = await context({ ...options, plugins: [report] });
  await ctx.watch();
} else {
  finish((await build(options)).metafile);
}
