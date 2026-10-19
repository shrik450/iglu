// Stryker: which console code is mutated, and how each mutant is tested.
// `just mutants` and `just mutants-tier console` run it; DEVELOPMENT.md
// explains what gates pull requests.

/** @type {import('@stryker-mutator/api/core').PartialStrykerOptions} */
export default {
  // The tests run under `node --test`, which Stryker has no runner for, so
  // each mutant runs the whole suite; it takes about a second.
  testRunner: "command",
  commandRunner: {
    command: "node --test 'src/**/*.test.ts' 'vendor/wterm-dom/src/**/*.test.ts'",
  },
  coverageAnalysis: "off",
  // Code the unit tests import. Left out, because only a browser runs them
  // and `just dev-check` covers them: store.ts, the signals that hold the
  // console's state over window.navigation and timers, and prefs.ts, which
  // keeps settings in localStorage.
  mutate: [
    "src/state/**/*.ts",
    "!src/state/**/*.test.ts",
    "!src/state/store.ts",
    "!src/state/prefs.ts",
    "vendor/wterm-dom/src/wheel.ts",
  ],
  concurrency: 2,
  // Scratch and reports in .dev/, as for the Rust mutants.
  tempDirName: "../.dev/tmp/stryker",
  cleanTempDir: "always",
  reporters: ["clear-text", "progress", "html", "json"],
  htmlReporter: { fileName: "../.dev/mutants/console/mutation.html" },
  jsonReporter: { fileName: "../.dev/mutants/console/mutation.json" },
  thresholds: { high: 90, low: 70, break: null },
};
