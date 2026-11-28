import { defineConfig } from "vitest/config";
import { NODE_TESTS } from "./vitest.config.ts";

// Stryker runs only the node tests (scripts/mutation/stryker.config.mjs, GEL-388).
export default defineConfig({ test: NODE_TESTS });
