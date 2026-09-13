// typescript-eslint 8.x has no TS 7 API (see issue 10940). Redirect
// `require("typescript")` to the TS 6 compatibility package for lint only.
// `tsc` / Vite keep using typescript@7.0.2.
const Module = require("node:module");
const { createRequire } = require("node:module");

const resolveHere = createRequire(__filename);
const typescript6 = resolveHere.resolve("@typescript/typescript6");
const original = Module._resolveFilename;

Module._resolveFilename = function patched(request, parent, isMain, options) {
  if (request === "typescript") {
    return typescript6;
  }
  return original.call(this, request, parent, isMain, options);
};
