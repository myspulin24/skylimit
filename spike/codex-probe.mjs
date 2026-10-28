// Spike T1: live probe of the official `codex app-server` JSON-RPC surface.
// Auth stays inside Codex; this script never reads ~/.codex/auth.json.
// Writes anonymized fixtures for usage-core tests.
import { spawn } from "node:child_process";
import { mkdirSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const outDir = join(here, "..", "crates", "usage-core", "tests", "fixtures", "codex");
mkdirSync(outDir, { recursive: true });

const child = spawn("codex", ["app-server"], { shell: true, stdio: ["pipe", "pipe", "pipe"] });
const pending = new Map();
let nextId = 1;
let buf = "";

child.stdout.on("data", (chunk) => {
  buf += chunk.toString("utf8");
  let nl;
  while ((nl = buf.indexOf("\n")) >= 0) {
    const line = buf.slice(0, nl).trim();
    buf = buf.slice(nl + 1);
    if (!line) continue;
    let msg;
    try { msg = JSON.parse(line); } catch { console.log("NON-JSON:", line.slice(0, 200)); continue; }
    if (msg.id != null && pending.has(msg.id)) {
      pending.get(msg.id)(msg);
      pending.delete(msg.id);
    } else if (msg.method) {
      console.log("NOTIFY", msg.method);
    }
  }
});
child.stderr.on("data", (c) => process.stderr.write("[stderr] " + c.toString("utf8").slice(0, 300)));

function send(method, params) {
  const id = nextId++;
  child.stdin.write(JSON.stringify({ jsonrpc: "2.0", id, method, params }) + "\n");
  return new Promise((resolve) => pending.set(id, resolve));
}
function notify(method, params) {
  child.stdin.write(JSON.stringify({ jsonrpc: "2.0", method, ...(params === undefined ? {} : { params }) }) + "\n");
}

// Remove anything identifying: ids, names, paths, previews, emails, balances.
function scrub(value, key = "") {
  if (Array.isArray(value)) return value.map((v) => scrub(v, key));
  if (value && typeof value === "object") {
    const out = {};
    for (const [k, v] of Object.entries(value)) out[k] = scrub(v, k);
    return out;
  }
  if (typeof value === "string") {
    if (/^(accountId|id|threadId|sessionId|projectId|parentThreadId|forkedFromId|limitId)$/.test(key)) return `<${key}>`;
    if (/^(name|preview|cwd|path|title|description|agentNickname|email|balance|sectionId)$/i.test(key)) return `<${key}>`;
  }
  return value;
}

const timer = setTimeout(() => { console.log("TIMEOUT"); child.kill(); process.exit(2); }, 30000);

const init = await send("initialize", { clientInfo: { name: "ai-usage-spike", version: "0.0.1" } });
console.log("initialize:", init.error ? JSON.stringify(init.error) : "ok");
notify("initialized");

const limits = await send("account/rateLimits/read", null);
writeFileSync(join(outDir, "rate-limits.json"), JSON.stringify(scrub(limits), null, 2));
console.log("rateLimits:", JSON.stringify(scrub(limits.result?.rateLimits ?? limits.error)).slice(0, 600));

const usage = await send("account/usage/read", null);
writeFileSync(join(outDir, "usage-account.json"), JSON.stringify(scrub(usage), null, 2));
const u = usage.result;
console.log("usage summary:", JSON.stringify(u?.summary ?? usage.error));
console.log("daily buckets:", u?.dailyUsageBuckets?.length ?? 0, "last:", JSON.stringify(u?.dailyUsageBuckets?.slice(-3)));

const threads = await send("thread/list", { limit: 5, useStateDbOnly: true });
const list = threads.result?.data ?? [];
console.log("threads:", list.length, threads.error ? JSON.stringify(threads.error) : "", "nextCursor:", !!threads.result?.nextCursor);
console.log("thread[0] keys:", list[0] ? Object.keys(list[0]).join(",") : "-", "createdAt:", list[0]?.createdAt, "updatedAt:", list[0]?.updatedAt, "model:", list[0]?.model, "source:", JSON.stringify(list[0]?.source), "originator:", list[0]?.originator);
// thread/list is only printed above, never saved: it carries git remotes,
// commit hashes and thread ids of real projects.

if (list[0]) {
  const tu = await send("account/usage/read", { threadId: list[0].id });
  writeFileSync(join(outDir, "usage-thread.json"), JSON.stringify(scrub(tu), null, 2));
  console.log("thread usage:", JSON.stringify(scrub(tu.result?.threadUsage ?? tu.error)).slice(0, 700));
}

clearTimeout(timer);
child.kill();
process.exit(0);
