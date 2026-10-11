import test from "node:test";
import assert from "node:assert/strict";
import {
  arrows,
  beforeChoices,
  CARD_H,
  CARD_W,
  changedFields,
  closesCycle,
  emptyPlan,
  GAP_X,
  instructionChanged,
  layout,
  resultGlyphs,
  resultsOf,
  shownSteps,
  suggestions,
  waves,
  type PlanDoc,
  type PlanStep,
} from "./planGraph.ts";

const step = (id: string, after: string[] = [], more: Partial<PlanStep> = {}): PlanStep => ({
  id,
  title: id,
  instruction: "",
  after,
  inputs: [],
  outputs: [],
  done_when: "",
  agent: "",
  approval: "none",
  on_fail: "stop",
  by: "human",
  ...more,
});

/** s1 -> s2, s1 -> s3, (s2, s3) -> s4. */
const diamond = () => [step("s1"), step("s2", ["s1"]), step("s3", ["s1"]), step("s4", ["s2", "s3"])];

test("waves follow the longest way back, steps in plan order", () => {
  assert.deepEqual(waves(diamond()), [["s1"], ["s2", "s3"], ["s4"]]);
  // A long side and a short side: the short one waits for the long one's end.
  const steps = [step("a"), step("b", ["a"]), step("c", ["b"]), step("d", ["a", "c"])];
  assert.deepEqual(waves(steps), [["a"], ["b"], ["c"], ["d"]]);
  assert.deepEqual(waves([]), []);
});

test("a wait on a step that is gone is ignored, and a cycle still ends", () => {
  assert.deepEqual(waves([step("s1", ["s9"])]), [["s1"]]);
  const loop = [step("a", ["b"]), step("b", ["a"])];
  assert.equal(waves(loop).flat().length, 2);
});

test("the arrows run from the step waited for to the one waiting", () => {
  assert.deepEqual(arrows(diamond()), [
    { from: "s1", to: "s2" },
    { from: "s1", to: "s3" },
    { from: "s2", to: "s4" },
    { from: "s3", to: "s4" },
  ]);
});

test("the graph puts a column per wave and centres the shorter columns", () => {
  const at = layout(diamond());
  assert.equal(at.get("s1")!.x, 0);
  assert.equal(at.get("s2")!.x, CARD_W + GAP_X);
  assert.equal(at.get("s4")!.x, 2 * (CARD_W + GAP_X));
  assert.equal(at.get("s2")!.x, at.get("s3")!.x, "one wave, one column");
  assert.notEqual(at.get("s2")!.y, at.get("s3")!.y, "side by side in it");
  const mid = (at.get("s2")!.y + at.get("s3")!.y) / 2;
  assert.equal(at.get("s1")!.y, mid, "a lone step sits level with the middle of the tallest column");
  assert.ok(Math.abs(at.get("s3")!.y - at.get("s2")!.y) >= CARD_H, "cards do not overlap");
});

test("the graph orders a column by where the steps waited for sit", () => {
  // x waits for b (lower), y for a (upper): y goes above x.
  const steps = [step("a"), step("b"), step("x", ["b"]), step("y", ["a"])];
  const at = layout(steps);
  assert.ok(at.get("y")!.y < at.get("x")!.y);
});

test("a step a person put somewhere stays there", () => {
  const steps = diamond();
  steps[3] = { ...steps[3], x: 999, y: -50 };
  const at = layout(steps);
  assert.deepEqual(at.get("s4"), { x: 999, y: -50 });
  assert.equal(at.get("s1")!.x, 0, "the rest are laid out as before");
});

test("a wait that would close a cycle is closed in the checklist", () => {
  const steps = diamond();
  assert.ok(closesCycle(steps, "s1", "s4"), "s1 cannot wait for s4, which waits for it");
  assert.ok(closesCycle(steps, "s2", "s2"), "nor for itself");
  assert.ok(!closesCycle(steps, "s3", "s2"), "s3 may wait for s2: s2 does not wait for s3");

  const forS2 = beforeChoices(steps, "s2");
  assert.deepEqual(
    forS2.map((c) => [c.id, c.on, c.closed]),
    [
      ["s1", true, false],
      ["s3", false, false],
      ["s4", false, true],
    ],
  );
  // A ticked wait is never closed, so it can always be taken away.
  assert.ok(beforeChoices(steps, "s4").every((c) => !c.closed));
  assert.deepEqual(beforeChoices(steps, "nope"), []);
});

const withSuggestions = (): PlanDoc => {
  const steps = diamond();
  const s5 = step("s5", ["s2"], { by: "agent" });
  const s4before = steps[3];
  const s4after = { ...s4before, after: ["s3", "s5"] };
  const gone = step("s9", ["s1"]);
  return {
    ...emptyPlan(),
    steps: [...steps.slice(0, 3), s4after, s5],
    changes: [
      { id: 10, target: "s5", run: "t1", before: null, after: s5 },
      { id: 11, target: "s4", run: "t1", before: s4before, after: s4after },
      { id: 12, target: "s9", run: "t1", before: gone, after: null },
    ],
  };
};

test("each suggestion says what it did to its step", () => {
  const doc = withSuggestions();
  const s = suggestions(doc);
  assert.equal(s.get("s5")!.kind, "added");
  assert.equal(s.get("s4")!.kind, "changed");
  assert.equal(s.get("s9")!.kind, "removed");
  assert.equal(s.has("s1"), false, "a step nobody suggested anything for has none");
  assert.deepEqual(changedFields(s.get("s4")!.change), ["after"]);
});

test("a step the agent removed is still shown, struck, until reviewed", () => {
  const shown = shownSteps(withSuggestions());
  assert.deepEqual(
    shown.map((x) => [x.step.id, x.removed]),
    [
      ["s1", false],
      ["s2", false],
      ["s3", false],
      ["s4", false],
      ["s5", false],
      ["s9", true],
    ],
  );
  // Kept or reverted, it is no longer a suggestion and no longer shown.
  const settled = { ...withSuggestions(), changes: [] };
  assert.equal(shownSteps(settled).some((x) => x.removed), false);
  assert.equal(suggestions(settled).size, 0);
});

test("the instruction moving on is noticed, whitespace aside", () => {
  const doc = { ...emptyPlan(), source: { instruction: "check deps", at: 1 } };
  assert.equal(instructionChanged(doc, " check deps\n"), false);
  assert.equal(instructionChanged(doc, "check deps and advisories"), true);
  assert.equal(instructionChanged(emptyPlan(), "anything"), false, "a plan with no source says nothing");
});

test("a run's step results read what is usable and leave the rest", () => {
  const r = resultsOf([
    { id: "s1", status: "done" },
    { id: "s2", status: "failed", note: "no network" },
    { id: "s3", status: "maybe" },
    { status: "done" },
    null,
  ]);
  assert.deepEqual([...r.keys()], ["s1", "s2"]);
  assert.equal(r.get("s2")!.note, "no network");
  assert.equal(resultsOf("nonsense").size, 0);
  assert.equal(resultGlyphs(["s1", "s2", "s3"], r), "✓✗–", "a step it said nothing of reads as skipped");
});
