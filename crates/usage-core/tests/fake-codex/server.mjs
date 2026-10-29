// Fake `codex app-server` for the usage-core E2E test.
//
// Speaks newline-delimited JSON-RPC 2.0 on stdio and answers with the
// anonymized live fixtures from the spike (tests/fixtures/codex). Never
// touches the network or any Codex login data.
//
// Arguments: --fixtures <dir> --control <file> --launches <file> --requests <file> --events <file>
// The control file is JSON, re-read on every request and every 100 ms:
//   die                 true → exit(3) now, and right after every start
//   primaryUsedPercent  overrides rateLimits.primary.usedPercent
//   pushUpdate          a counter; each increase sends account/rateLimits/updated
import { appendFileSync, readFileSync } from "node:fs";
import { join } from "node:path";

const args = {};
for (let i = 2; i < process.argv.length; i += 2) args[process.argv[i].replace(/^--/, "")] = process.argv[i + 1];

const control = () => {
  try {
    return JSON.parse(readFileSync(args.control, "utf8"));
  } catch {
    return {};
  }
};
const record = (file, text) => {
  if (file) appendFileSync(file, text + "\n");
};

record(args.launches, String(process.pid));
record(args.events, JSON.stringify({ pid: process.pid, event: "start", at: Date.now() }));
process.on("exit", (code) => record(args.events, JSON.stringify({ pid: process.pid, event: "exit", code, at: Date.now() })));
if (control().die) process.exit(3);

const fixture = (name) => JSON.parse(readFileSync(join(args.fixtures, name), "utf8")).result;
const rateLimits = fixture("rate-limits.json");
const usage = fixture("usage-account.json");

const send = (msg) => process.stdout.write(JSON.stringify(msg) + "\n");

let pushed = control().pushUpdate ?? 0;
setInterval(() => {
  const c = control();
  if (c.die) process.exit(3);
  if ((c.pushUpdate ?? 0) > pushed) {
    pushed = c.pushUpdate;
    send({ jsonrpc: "2.0", method: "account/rateLimits/updated", params: {} });
  }
}, 100);

let buf = "";
process.stdin.setEncoding("utf8");
process.stdin.on("data", (chunk) => {
  buf += chunk;
  let nl;
  while ((nl = buf.indexOf("\n")) >= 0) {
    const line = buf.slice(0, nl).trim();
    buf = buf.slice(nl + 1);
    if (!line) continue;
    let msg;
    try {
      msg = JSON.parse(line);
    } catch {
      continue;
    }
    handle(msg);
  }
});
process.stdin.on("end", () => process.exit(0));

function handle(msg) {
  record(args.requests, JSON.stringify({ method: msg.method, hasId: msg.id != null, params: msg.params ?? null }));
  const c = control();
  switch (msg.method) {
    case "initialize":
      send({ id: msg.id, result: { userAgent: "fake-codex/0.0.0" } });
      // Notifications the client must ignore.
      send({ jsonrpc: "2.0", method: "remoteControl/status/changed", params: { status: "disabled" } });
      break;
    case "initialized":
      send({ jsonrpc: "2.0", method: "account/updated", params: { authMode: "chatgpt" } });
      break;
    case "account/rateLimits/read": {
      const r = structuredClone(rateLimits);
      if (typeof c.primaryUsedPercent === "number") r.rateLimits.primary.usedPercent = c.primaryUsedPercent;
      send({ id: msg.id, result: r });
      break;
    }
    case "account/usage/read":
      send({ id: msg.id, result: usage });
      break;
    case "account/read":
      send({
        id: msg.id,
        result: {
          account: { type: "chatgpt", email: "tester@example.com", planType: "team", workspaceName: "Test" },
          requiresOpenaiAuth: true,
        },
      });
      break;
    default:
      if (msg.id != null) send({ id: msg.id, error: { code: -32601, message: "method not found" } });
  }
}
