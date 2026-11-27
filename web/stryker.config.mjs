// Mutation testing of the web client (GEL-388), run by
// .github/workflows/mutation-web.yml on the lines changed since the latest
// release. By hand, name what to mutate, or it mutates all of src/:
//   npx stryker run --mutate src/messages/markdown.ts:10-40 --concurrency 2
/** @type {import('@stryker-mutator/api/core').PartialStrykerOptions} */
export default {
  testRunner: "vitest",
  vitest: { configFile: "vitest.mutation.config.ts" },
  coverageAnalysis: "perTest",
  // Stryker rewrites the tsconfig with ts.parseConfigFileTextToJson, which
  // TypeScript 7 no longer has; ours needs no rewrite, so it gets no file.
  tsconfigFile: "tsconfig.stryker-none.json",
  // The workflow mutates only changed lines; it keeps no state between runs.
  incremental: false,
  // Changed lines that no node test imports are reported, not an error.
  allowEmpty: true,
  reporters: ["json", "html", "progress-append-only"],
};
