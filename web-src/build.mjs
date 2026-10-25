// Bundles src/terminal.ts and src/editor.ts into the app's assets (PLAN D9, ADR 0004).
// Run through scripts/build-web.sh. The output is deterministic: no source maps,
// no timestamps, entry names fixed, so a second run changes nothing.

import { build } from "esbuild";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";

const here = dirname(fileURLToPath(import.meta.url));
const outdir = join(here, "..", "crates", "berth-app", "assets", "web");

await build({
  absWorkingDir: here,
  entryPoints: { terminal: "src/terminal.ts", editor: "src/editor.ts" },
  outdir,
  bundle: true,
  format: "iife",
  platform: "browser",
  target: "safari17",
  minify: true,
  legalComments: "none",
  charset: "utf8",
  sourcemap: false,
  logLevel: "info",
});
