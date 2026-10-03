/** A turn's steps piling up and folding away: `node --test`. */

import { strict as assert } from "node:assert";
import { test } from "node:test";
import { detail, isActive, isOpen, RECENT, shape, span, summarize, took, verbKey, visible, type StepSegment, type StepTool } from "./steps.ts";

const tool = (id: string, over: Partial<StepTool> = {}): StepSegment => ({
  kind: "tool",
  tool: { id, title: `t${id}`, toolKind: "read", status: "completed", ...over },
});
const text = (s: string): StepSegment => ({ kind: "text", text: s });
const thought = (s: string): StepSegment => ({ kind: "thought", text: s });

test("consecutive calls pile up in one block instead of replacing each other", () => {
  const blocks = shape([tool("1"), tool("2"), tool("3")]);
  assert.equal(blocks.length, 1);
  assert.equal(blocks[0].kind, "steps");
  assert.deepEqual(
    blocks[0].kind === "steps" && blocks[0].steps.map((s) => (s.kind === "tool" ? s.tool.id : "")),
    ["1", "2", "3"],
  );
});

test("prose splits the steps, and each block keeps its place among the blocks", () => {
  const blocks = shape([tool("1"), text("looked"), tool("2"), thought("hmm"), tool("3"), text("done")]);
  assert.deepEqual(
    blocks.map((b) => (b.kind === "steps" ? `steps@${b.at}:${b.steps.length}` : "text")),
    ["steps@0:1", "text", "steps@1:3", "text"],
  );
});

test("empty prose and empty thoughts neither show nor split the steps", () => {
  const blocks = shape([tool("1"), text("  \n"), thought("   "), tool("2")]);
  assert.equal(blocks.length, 1);
  assert.equal(blocks[0].kind === "steps" && blocks[0].steps.length, 2);
  // The caller decides what counts as empty: hook chatter, here.
  const hooked = shape([tool("1"), text("Notice: x says: y"), tool("2")], (s) => !s.startsWith("Notice:"));
  assert.equal(hooked.length, 1);
});

test("the block being added to is open; it folds once the answer starts or the turn ends", () => {
  const working = shape([text("let me look"), tool("1"), tool("2")]);
  assert.equal(isActive(working, 1, true), true);
  assert.equal(isOpen(isActive(working, 1, true), undefined), true);

  const answering = shape([text("let me look"), tool("1"), tool("2"), text("found it")]);
  assert.equal(isActive(answering, 1, true), false);
  assert.equal(isOpen(isActive(answering, 1, true), undefined), false);

  // The turn ended on a call (stopped, or failed): nothing is active.
  assert.equal(isOpen(isActive(working, 1, false), undefined), false);
});

test("the human's choice wins either way", () => {
  assert.equal(isOpen(false, true), true, "a folded summary clicked open stays open");
  assert.equal(isOpen(true, false), false, "a live list folded by hand stays folded");
});

test("a long list shows the newest few and counts the rest", () => {
  const many = shape(Array.from({ length: RECENT + 4 }, (_, i) => tool(String(i))));
  const steps = many[0].kind === "steps" ? many[0].steps : [];
  const v = visible(steps, false);
  assert.equal(v.hidden, 4);
  assert.equal(v.shown.length, RECENT);
  assert.equal(v.shown[0].kind === "tool" && v.shown[0].tool.id, "4");
  assert.equal(visible(steps, true).hidden, 0);
  assert.equal(visible(steps.slice(0, 3), false).hidden, 0);
});

test("the verb is ongoing only while the call runs in a live turn", () => {
  const live = { kind: "tool" as const, tool: { id: "1", title: "", toolKind: "search", status: "in_progress" } };
  assert.equal(verbKey(live, true), "steps.verb.search.now");
  assert.equal(verbKey(live, false), "steps.verb.search.done", "a stopped turn ends its calls too");
  assert.equal(verbKey({ kind: "tool", tool: { ...live.tool, status: "completed" } }, true), "steps.verb.search.done");
  assert.equal(verbKey({ kind: "tool", tool: { ...live.tool, toolKind: "mystery" } }, true), "steps.verb.other.now");
  assert.equal(verbKey({ kind: "thought", text: "x" }, true), "steps.verb.think.done");
});

test("the detail drops what the verb already says", () => {
  const d = (title: string) => detail({ id: "1", title, toolKind: "other", status: "completed" });
  assert.equal(d("Read src/lib/a.ts"), "src/lib/a.ts");
  assert.equal(d("`npm test`"), "npm test");
  assert.equal(d("mcp__divixi__spawn_worker"), "spawn_worker");
  assert.equal(d("grep -n foo"), "grep -n foo");
});

test("the summary counts calls and failures and spans first to last", () => {
  const s = shape([
    tool("1", { startedMs: 100, endedMs: 400 }),
    thought("why"),
    tool("2", { status: "failed", startedMs: 450, endedMs: 2600 }),
  ]);
  const steps = s[0].kind === "steps" ? s[0].steps : [];
  assert.deepEqual(summarize(steps), { tools: 2, failed: 1, ms: 2500 });
  assert.equal(summarize([{ kind: "thought", text: "x" }]).tools, 0);
  assert.equal(summarize([{ kind: "thought", text: "x" }]).ms, undefined);
});

test("one call's time shows only once it is over", () => {
  assert.equal(took({ id: "1", title: "", toolKind: "read", status: "completed", startedMs: 10, endedMs: 1210 }), 1200);
  assert.equal(took({ id: "1", title: "", toolKind: "read", status: "in_progress", startedMs: 10 }), undefined);
  assert.equal(span(840), "840ms");
  assert.equal(span(1200), "1.2s");
  assert.equal(span(125_000), "2m 5s");
});
