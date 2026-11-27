/* global console, process */
// The browser checks that need the real API, its gateway and the SFU, one
// after another against one running app: GELABBER_SMOKE_URL, a Vite dev
// server whose /api and /media reach them. e2e-extended.yml runs them after
// merge; each one that runs past its time is killed and counted as failed.
import { mkdir } from "node:fs/promises";
import { runWithin } from "../run-within.mjs";

const url = process.env.GELABBER_SMOKE_URL ?? "http://127.0.0.1:5174";
const reports =
  process.env.GELABBER_SERVER_CHECKS_REPORT ?? "/tmp/gelabber-server-checks";
const LIMIT_S = 480;
const scripts = [
  ["e2e/chat-workflows.mjs", { GELABBER_CHAT_REPORT: `${reports}/chat` }],
  ["e2e/search-context.mjs", { GELABBER_SEARCH_REPORT: `${reports}/search` }],
  [
    "e2e/webapp-accessibility.mjs",
    { GELABBER_ACCESSIBILITY_REPORT: `${reports}/accessibility` },
  ],
  [
    "smoke-reactions.mjs",
    {
      GELABBER_REACTIONS_URL: url,
      GELABBER_REACTIONS_OUTPUT: `${reports}/reactions`,
    },
  ],
  ["smoke-call-sounds.mjs", { GELABBER_CALL_SOUNDS_URL: url }],
  ["smoke-audio-processing.mjs", { GELABBER_AUDIO_URL: url }],
  [
    "smoke-source-audio.mjs",
    {
      GELABBER_SOURCE_AUDIO_URL: url,
      GELABBER_SOURCE_AUDIO_REPORT: `${reports}/source-audio.json`,
    },
  ],
];

await mkdir(reports, { recursive: true });
const failed = [];
for (const [script, env] of scripts) {
  const args = [`scripts/${script}`];
  const result = await runWithin(args, {
    limitMs: LIMIT_S * 1000,
    env: { GELABBER_SMOKE_URL: url, ...env },
  });
  console.log(
    `--- node ${script}${result.killed ? `: killed after ${LIMIT_S} s` : ""}`,
  );
  process.stdout.write(result.output);
  if (result.killed || result.code !== 0) failed.push(script);
}
for (const script of failed) console.error(`Failed: node scripts/${script}`);
process.exit(failed.length ? 1 : 0);
