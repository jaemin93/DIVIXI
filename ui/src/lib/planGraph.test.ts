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
  needsFirstDraft,
  nextRunsLimit,
  openNeedsConfirm,
  sameVersion,
  runView,
  resultGlyphs,
  resultsOf,
  shownSteps,
  suggestions,
  unsaved,
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

test("a pin that the order of steps has overtaken is let go (lay out after a change of order)", () => {
  // s4 was put by hand right of s2 and s3. Then its steps before changed:
  // it now waits for s5, which sits further right. The old place would put
  // it before a step it waits for, so the layout places it after s5.
  const steps = [step("s1"), step("s2", ["s1"]), step("s3", ["s1"]), step("s5", ["s2"]), step("s4", ["s3", "s5"], { x: 2 * (CARD_W + GAP_X), y: 0 })];
  const at = layout(steps);
  assert.ok(at.get("s4")!.x > at.get("s5")!.x, `s4 at ${at.get("s4")!.x}, s5 at ${at.get("s5")!.x}`);
  // A pin that still reads in order stays.
  const kept = [step("s1"), step("s2", ["s1"], { x: 500, y: 7 })];
  assert.deepEqual(layout(kept).get("s2"), { x: 500, y: 7 });
  // One left of a step that waits for it is let go too.
  const early = [step("s1", [], { x: 900, y: 0 }), step("s2", ["s1"], { x: 100, y: 0 })];
  const e = layout(early);
  assert.ok(e.get("s1")!.x < e.get("s2")!.x);
});

test("a change in a step's words is unsaved; a move on the graph is not", () => {
  const steps = diamond();
  const doc = { ...emptyPlan(), steps, saved: [{ n: 1, at: 1, note: "", steps }] };
  assert.equal(unsaved(doc), false);
  assert.equal(unsaved({ ...doc, steps: steps.map((s) => (s.id === "s2" ? { ...s, x: 40, y: 9 } : s)) }), false);
  assert.equal(unsaved({ ...doc, steps: steps.map((s) => (s.id === "s2" ? { ...s, inputs: ["repo"] } : s)) }), true);
  assert.equal(unsaved({ ...doc, steps: steps.map((s) => (s.id === "s4" ? { ...s, after: ["s3"] } : s)) }), true);
  assert.equal(unsaved({ ...emptyPlan(), steps }), true, "never saved, with steps");
  assert.equal(unsaved(emptyPlan()), false, "never saved, empty");
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

test("a first draft is asked for once: a routine with no plan, an empty plan, nothing under way", () => {
  const empty = emptyPlan();
  assert.equal(needsFirstDraft({ hadPlan: false, doc: empty, asked: false, busy: false }), true);
  assert.equal(needsFirstDraft({ hadPlan: true, doc: empty, asked: false, busy: false }), false, "a routine that had a plan: just show it");
  assert.equal(needsFirstDraft({ hadPlan: false, doc: empty, asked: true, busy: false }), false, "asked already (pressed again)");
  assert.equal(needsFirstDraft({ hadPlan: false, doc: empty, asked: false, busy: true }), false, "a turn under way (came back to the page)");
  const drafted = { ...empty, steps: diamond() };
  assert.equal(needsFirstDraft({ hadPlan: false, doc: drafted, asked: false, busy: false }), false, "it has steps");
  const suggested = { ...empty, changes: [{ id: 1, target: "s1", run: null, before: null, after: step("s1") }] };
  assert.equal(needsFirstDraft({ hadPlan: false, doc: suggested, asked: false, busy: false }), false, "suggestions are on their way");
});

test("a run in flight shows where its agent said it is", () => {
  const live = {
    run: "rr1",
    plan: { version: 3, steps: diamond().map((s) => ({ id: s.id, title: s.title, after: s.after })) },
    progress: { s1: { status: "done" as const, note: "" }, s2: { status: "started" as const, note: "" } },
  };
  const v = runView(diamond(), live)!;
  assert.deepEqual(
    [...v.states.entries()],
    [
      ["s1", "done"],
      ["s2", "running"],
      ["s3", "waiting"],
      ["s4", "waiting"],
    ],
  );
  assert.equal(v.told, true);
  // Nothing said yet: the run is on, nothing more is known.
  const quiet = runView(diamond(), { ...live, progress: {} })!;
  assert.equal(quiet.told, false);
  // No run, or a run given no plan: nothing to show.
  assert.equal(runView(diamond(), null), null);
  assert.equal(runView(diamond(), { ...live, plan: null }), null);
});

test("a plan edited during a run is shown as the run has it", () => {
  const live = { run: "rr1", plan: { version: 3, steps: diamond().map((s) => ({ id: s.id, title: s.title, after: s.after })) }, progress: {} };
  // Meanwhile: s2 renamed and moved after s3, s3 deleted, s5 added, a card placed by hand.
  const working = [step("s1"), step("s2", ["s1"], { title: "renamed", x: 500, y: 9 }), step("s4", ["s2"]), step("s5", ["s4"])];
  const v = runView(working, live)!;
  assert.deepEqual(v.steps.map((s) => s.id), ["s1", "s2", "s3", "s4"], "the run's steps, not the working plan's");
  assert.equal(v.steps.find((s) => s.id === "s2")!.title, "renamed", "words from the working plan where the step is still there");
  assert.equal(v.steps.find((s) => s.id === "s3")!.title, "s3", "and the run's own title where it is not");
  assert.deepEqual(v.steps.find((s) => s.id === "s4")!.after, ["s2", "s3"], "the run's waits");
  assert.equal(v.steps.find((s) => s.id === "s2")!.x, undefined, "laid out as the run's graph");
});

test("opening a version asks first only when the working plan is in no saved version", () => {
  const v1 = diamond();
  const v2 = [...diamond(), step("s5", ["s4"])];
  const doc = { ...emptyPlan(), steps: v2, saved: [{ n: 1, at: 1, note: "", steps: v1 }, { n: 2, at: 2, note: "", steps: v2 }] };
  assert.equal(sameVersion(doc), 2, "the working plan is v2");
  assert.equal(openNeedsConfirm(doc, 1), false, "v2 is saved: nothing is lost");
  assert.equal(openNeedsConfirm(doc, 2), false, "already v2");
  // A card moved by hand is no change.
  assert.equal(openNeedsConfirm({ ...doc, steps: v2.map((s) => ({ ...s, x: 3, y: 4 })) }, 1), false);
  // Edited past every version: opening one would drop the edit.
  const edited = { ...doc, steps: v2.map((s) => (s.id === "s1" ? { ...s, title: "pull first" } : s)) };
  assert.equal(sameVersion(edited), undefined);
  assert.equal(openNeedsConfirm(edited, 1), true);
  // Working plan back at v1 (opened earlier): opening v2 loses nothing.
  assert.equal(openNeedsConfirm({ ...doc, steps: v1 }, 2), false);
  // Nothing on the working plan at all: nothing to lose.
  assert.equal(openNeedsConfirm({ ...doc, steps: [] }, 1), false);
});

test("the history menu reads more runs until a page comes back short", () => {
  assert.equal(nextRunsLimit(20, 20), 40, "a full page: there may be more");
  assert.equal(nextRunsLimit(40, 40, 10), 50);
  assert.equal(nextRunsLimit(13, 20), null, "a short page: that is all of them");
  assert.equal(nextRunsLimit(0, 20), null);
});
