#!/usr/bin/env node
// A scripted agent that speaks ACP, for checking the app end to end
// without spending a token or waiting on a model.
//
//   DIVIXI_ACP_ADAPTER=$PWD/scripts/fake-acp-agent.mjs divixi-server serve
//
// With that variable set, every Claude Code session the app opens runs this
// script instead of the Claude adapter. It answers `initialize`,
// `session/new` and `session/load` (it claims it can load, and does nothing
// on a load, so a reopened worker "resumes"), and on `session/prompt` it
// does what a ```fake block in the prompt says, one line at a time:
//
//   say <text>                  a message chunk
//   think <text>                a thought chunk
//   tool <seconds> <title>      a tool call (kind execute) that takes that long
//   edit <path>                 a tool call of kind edit naming that file
//   run <shell command>         runs it (sh -c) as a tool call; its stdout is the output
//   sleep <seconds>             says nothing for that long
//   ask <title>                 asks permission (execute) and says which option came back
//   mcp <tool> <json args>      calls that tool on the divixi MCP server the session
//                               was given (a conductor's spawn_worker, say) and says
//                               what came back; "@@FILE:/path" inside the args is
//                               replaced with that file's text, so a worker's own
//                               ```fake block can be handed over without nesting
//   report <json>               ends the reply with that ```divixi-report block
//   fail <text>                 ends the turn with an error
//
// A prompt without a block gets "ok" and a report that says done. The
// prompt the conductor sends arrives whole, so the script a worker is to
// follow goes in the task text; what the app appends after it (the folder
// line, the report rules) is ignored.
//
// With FAKE_ACP_LOG set to a file, every prompt the agent is given is
// appended to it whole, which is how a check reads what the app put in
// front of an agent (the preamble, a `[divixi]` note) -- none of that is
// in the record.
//
// Not a test of any agent: a test of what the app does around one.

import * as acp from "@agentclientprotocol/sdk";
import { Readable, Writable } from "node:stream";
import { spawn } from "node:child_process";
import { appendFileSync, readFileSync } from "node:fs";

const sessions = new Map();
let calls = 0;
let rpc = 0;

/** The divixi MCP server a session was opened with, if any. */
function mcpOf(params) {
  const s = (params.mcpServers ?? []).find((m) => m.type === "http" || m.url);
  if (!s) return null;
  const headers = {};
  for (const h of s.headers ?? []) headers[h.name] = h.value;
  return { url: s.url, headers };
}

/** One tools/call on the MCP server; the text of its result. */
async function callTool(mcp, name, args) {
  if (!mcp) return "(no mcp server in this session)";
  const body = { jsonrpc: "2.0", id: ++rpc, method: "tools/call", params: { name, arguments: args } };
  const res = await fetch(mcp.url, { method: "POST", headers: { ...mcp.headers, "content-type": "application/json" }, body: JSON.stringify(body) });
  const json = await res.json();
  const text = json?.result?.content?.map((c) => c.text ?? "").join("") ?? JSON.stringify(json);
  return `${json?.result?.isError ? "error: " : ""}${text}`;
}

function text(params) {
  return (params.prompt ?? [])
    .map((b) => (b.type === "text" ? b.text : ""))
    .join("");
}

/** The lines of the first ```fake block in a prompt, or none. */
function script(prompt) {
  const m = /```fake\n([\s\S]*?)```/.exec(prompt);
  if (!m) return null;
  return m[1]
    .split("\n")
    .map((l) => l.trim())
    .filter((l) => l && !l.startsWith("#"));
}

const sleep = (ms, signal) =>
  new Promise((resolve, reject) => {
    const t = setTimeout(resolve, ms);
    signal?.addEventListener("abort", () => {
      clearTimeout(t);
      reject(new Error("cancelled"));
    });
  });

async function update(cx, sessionId, update) {
  await cx.notify(acp.methods.client.session.update, { sessionId, update });
}

async function say(cx, sessionId, text) {
  await update(cx, sessionId, { sessionUpdate: "agent_message_chunk", content: { type: "text", text } });
}

async function runLine(cx, sessionId, line, signal, cwd, mcp) {
  const [verb, ...rest] = line.split(" ");
  const arg = rest.join(" ");
  switch (verb) {
    case "mcp": {
      const [tool, ...json] = rest;
      let text = json.join(" ").replace(/@@FILE:(\S+?)(?=["\s])/g, (_, p) => JSON.stringify(readFileSync(p, "utf8")).slice(1, -1));
      const args = text.trim() ? JSON.parse(text) : {};
      const id = `call_${++calls}`;
      await update(cx, sessionId, { sessionUpdate: "tool_call", toolCallId: id, title: `mcp.divixi.${tool}`, kind: "other", status: "in_progress" });
      const out = await callTool(mcp, tool, args);
      await update(cx, sessionId, { sessionUpdate: "tool_call_update", toolCallId: id, status: "completed" });
      await say(cx, sessionId, `${tool} → ${out}\n`);
      return;
    }
    case "say":
      await say(cx, sessionId, arg + "\n");
      return;
    case "think":
      await update(cx, sessionId, { sessionUpdate: "agent_thought_chunk", content: { type: "text", text: arg } });
      return;
    case "sleep":
      await sleep(Number(arg) * 1000, signal);
      return;
    case "tool": {
      const [secs, ...t] = rest;
      const id = `call_${++calls}`;
      await update(cx, sessionId, { sessionUpdate: "tool_call", toolCallId: id, title: t.join(" "), kind: "execute", status: "in_progress" });
      await sleep(Number(secs) * 1000, signal);
      await update(cx, sessionId, { sessionUpdate: "tool_call_update", toolCallId: id, status: "completed" });
      return;
    }
    case "edit": {
      const id = `call_${++calls}`;
      await update(cx, sessionId, { sessionUpdate: "tool_call", toolCallId: id, title: `Write ${arg}`, kind: "edit", status: "in_progress", locations: [{ path: arg }] });
      await update(cx, sessionId, { sessionUpdate: "tool_call_update", toolCallId: id, status: "completed" });
      return;
    }
    case "run": {
      const id = `call_${++calls}`;
      await update(cx, sessionId, { sessionUpdate: "tool_call", toolCallId: id, title: arg, kind: "execute", status: "in_progress" });
      const out = await new Promise((resolve) => {
        const child = spawn("sh", ["-c", arg], { cwd, stdio: ["ignore", "pipe", "pipe"] });
        let buf = "";
        child.stdout.on("data", (d) => (buf += d));
        child.stderr.on("data", (d) => (buf += d));
        child.on("close", (code) => resolve(`${buf.trim()}${code ? `\n(exit ${code})` : ""}`));
        signal?.addEventListener("abort", () => child.kill("SIGKILL"));
      });
      await update(cx, sessionId, {
        sessionUpdate: "tool_call_update",
        toolCallId: id,
        status: "completed",
        content: [{ type: "content", content: { type: "text", text: out } }],
      });
      if (out) await say(cx, sessionId, out + "\n");
      return;
    }
    case "ask": {
      const id = `call_${++calls}`;
      const answer = await cx.request(acp.methods.client.session.requestPermission, {
        sessionId,
        toolCall: { toolCallId: id, title: arg, kind: "execute", status: "pending", rawInput: { command: arg } },
        options: [
          { kind: "allow_once", name: "Allow", optionId: "allow" },
          { kind: "reject_once", name: "Refuse", optionId: "reject" },
        ],
      });
      const got = answer.outcome.outcome === "cancelled" ? "cancelled" : answer.outcome.optionId;
      await say(cx, sessionId, `permission ${got}\n`);
      return;
    }
    case "report":
      await say(cx, sessionId, "\n```divixi-report\n" + arg + "\n```\n");
      return;
    case "fail":
      throw Object.assign(new Error(arg), { fail: true });
    default:
      await say(cx, sessionId, `(unknown line: ${line})\n`);
  }
}

const DONE = '{"status":"done","summary":"ok","changes":[],"checks":[],"risks":[],"questions":[],"next":[]}';

const agent = {
  async initialize() {
    return {
      protocolVersion: acp.PROTOCOL_VERSION,
      agentCapabilities: { loadSession: true, promptCapabilities: { image: false }, mcpCapabilities: { http: true } },
      agentInfo: { name: "fake-acp-agent", version: "0.0.1" },
    };
  },
  async newSession(params) {
    const sessionId = `fake-${Math.random().toString(36).slice(2, 10)}`;
    sessions.set(sessionId, { cwd: params.cwd, pending: null, mcp: mcpOf(params) });
    return { sessionId, modes: { currentModeId: "auto", availableModes: [{ id: "auto", name: "Auto" }, { id: "default", name: "Manual" }] } };
  },
  async loadSession(params) {
    sessions.set(params.sessionId, { cwd: params.cwd, pending: null, mcp: mcpOf(params) });
    return { modes: { currentModeId: "auto", availableModes: [{ id: "auto", name: "Auto" }, { id: "default", name: "Manual" }] } };
  },
  async prompt(params, cx) {
    const session = sessions.get(params.sessionId);
    if (!session) throw new Error(`no session ${params.sessionId}`);
    session.pending?.abort();
    const control = new AbortController();
    session.pending = control;
    const prompt = text(params);
    if (process.env.FAKE_ACP_LOG) {
      try {
        appendFileSync(process.env.FAKE_ACP_LOG, `---- ${new Date().toISOString()} session ${params.sessionId} cwd ${session.cwd}\n${prompt}\n`);
      } catch {
        // Nowhere to write: the log is a convenience.
      }
    }
    const lines = script(prompt);
    try {
      if (!lines) {
        await say(cx, params.sessionId, "ok\n\n```divixi-report\n" + DONE + "\n```\n");
      } else {
        for (const line of lines) {
          if (control.signal.aborted) break;
          await runLine(cx, params.sessionId, line, control.signal, session.cwd, session.mcp);
        }
      }
    } catch (err) {
      if (control.signal.aborted) return { stopReason: "cancelled" };
      if (err?.fail) throw acp.RequestError.internalError(err.message);
      throw err;
    } finally {
      session.pending = null;
    }
    return { stopReason: control.signal.aborted ? "cancelled" : "end_turn" };
  },
  async cancel(params) {
    sessions.get(params.sessionId)?.pending?.abort();
  },
};

const stream = acp.ndJsonStream(Writable.toWeb(process.stdout), Readable.toWeb(process.stdin));
acp
  .agent({ name: "fake-acp-agent" })
  .onRequest("initialize", (ctx) => agent.initialize(ctx.params))
  .onRequest("session/new", (ctx) => agent.newSession(ctx.params))
  .onRequest("session/load", (ctx) => agent.loadSession(ctx.params))
  .onRequest("authenticate", () => ({}))
  .onRequest("session/set_mode", () => ({}))
  .onRequest("session/set_config_option", () => ({}))
  .onRequest("session/prompt", (ctx) => agent.prompt(ctx.params, ctx.client))
  .onNotification("session/cancel", (ctx) => agent.cancel(ctx.params))
  .connect(stream);
