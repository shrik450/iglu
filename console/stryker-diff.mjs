// Mutates only the console code changed against a base, and fails when its
// tests miss a mutant: `node stryker-diff.mjs <base>`, from console/.
//
// The lines come from Git's diff, kept to what stryker.config.mjs mutates,
// and go to Stryker as ranges, so a change is judged by its own lines.

import { execFileSync, spawnSync } from "node:child_process";
import { readFileSync } from "node:fs";
import path from "node:path";

const base = process.argv[2] ?? "main";
const { default: config } = await import("./stryker.config.mjs");
const include = config.mutate.filter((g) => !g.startsWith("!"));
const exclude = config.mutate.filter((g) => g.startsWith("!")).map((g) => g.slice(1));
const inScope = (file) =>
  include.some((g) => path.matchesGlob(file, g)) && !exclude.some((g) => path.matchesGlob(file, g));

const diff = execFileSync("git", ["diff", "-U0", "--merge-base", base, "--relative", "--", "."], {
  encoding: "utf8",
  maxBuffer: 64 * 1024 * 1024,
});
const ranges = [];
let file = null;
for (const line of diff.split("\n")) {
  if (line.startsWith("+++ ")) {
    const name = line.slice(4);
    file = name === "/dev/null" ? null : name.replace(/^b\//, "");
  } else if (line.startsWith("@@") && file && inScope(file)) {
    const [, start, count = "1"] = line.match(/^@@ -\S+ \+(\d+)(?:,(\d+))? @@/) ?? [];
    if (start && Number(count) > 0) {
      ranges.push(`${file}:${start}-${Number(start) + Number(count) - 1}`);
    }
  }
}
if (ranges.length === 0) {
  console.log(`No console code that Stryker mutates changed against ${base}.`);
  process.exit(0);
}

const run = spawnSync("npx", ["stryker", "run", "--mutate", ranges.join(",")], { stdio: "inherit" });
if (run.status !== 0) process.exit(run.status ?? 1);

// Timeouts count as caught: the tests didn't pass. Survivors and mutants no
// test reached fail the change.
const report = JSON.parse(readFileSync(config.jsonReporter.fileName, "utf8"));
const missed = Object.entries(report.files).flatMap(([name, { mutants }]) =>
  mutants
    .filter((m) => m.status === "Survived" || m.status === "NoCoverage")
    .map((m) => ({ name, ...m })),
);
for (const m of missed) {
  const where = `console/${name(m)}:${m.location.start.line}`;
  const what = `${m.mutatorName} mutant ${m.status.toLowerCase()}: ${JSON.stringify(m.replacement ?? "")}`;
  console.log(process.env.GITHUB_ACTIONS ? `::error file=console/${name(m)},line=${m.location.start.line}::${what}` : `${where}: ${what}`);
}
console.log(`${missed.length} of the changed lines' mutants weren't caught.`);
process.exit(missed.length > 0 ? 1 : 0);

function name(m) {
  return path.relative(process.cwd(), path.resolve(m.name));
}
