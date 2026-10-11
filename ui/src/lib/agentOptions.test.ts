import test from "node:test";
import assert from "node:assert/strict";
import { choiceList, missingChoice, pickable, shownChoice } from "./agentOptions.ts";

const choice = (id: string, name = id, description: string | null = null) => ({ id, name, description, group: null });
const option = (id: string, category: string, current: string, choices: ReturnType<typeof choice>[]) => ({
  id,
  name: id,
  description: null,
  category,
  current,
  choices,
});

// Copilot CLI's model selector as a status saved before detection dropped
// the repeats: `auto` three times, every other model twice.
const copilotModel = option("model", "model", "claude-sonnet-4.6", [
  choice("auto", "Auto", "Let Copilot pick the best model"),
  choice("auto", "Auto", "Auto"),
  choice("claude-sonnet-4.6", "Claude Sonnet 4.6"),
  choice("gpt-5.4", "GPT-5.4"),
  choice("auto", "Auto", "Auto"),
  choice("claude-sonnet-4.6", "Claude Sonnet 4.6"),
  choice("gpt-5.4", "GPT-5.4"),
]);

test("a choice listed more than once is offered once, the first listing winning", () => {
  const [model] = pickable([copilotModel]);
  assert.deepEqual(
    model.choices.map((c) => c.id),
    ["auto", "claude-sonnet-4.6", "gpt-5.4"],
  );
  assert.equal(model.choices[0].description, "Let Copilot pick the best model");
  assert.equal(model.current, "claude-sonnet-4.6");
  assert.equal(model.id, "model");
});

test("an option without repeats comes back as it was", () => {
  const effort = option("reasoning_effort", "thought_level", "medium", [choice("low"), choice("medium"), choice("high")]);
  const [out] = pickable([effort]);
  assert.equal(out, effort);
});

test("options with nothing to pick are left out", () => {
  const flag = option("fast", "other", "true", []);
  assert.deepEqual(
    pickable([flag, copilotModel]).map((o) => o.id),
    ["model"],
  );
});

test("the agent's list itself is not changed", () => {
  pickable([copilotModel]);
  assert.equal(copilotModel.choices.length, 7);
});

test("a model list has the agent's choices only, no agent-default entry", () => {
  const m = option("model", "model", "sonnet", [choice("opus"), choice("sonnet"), choice("haiku")]);
  const list = choiceList(m, undefined);
  assert.deepEqual(
    list.map((c) => c.id),
    ["opus", "sonnet", "haiku"],
  );
  assert.ok(list.every((c) => c.id !== ""), "no empty (default) choice");
});

test("nothing saved shows the agent's current model as picked", () => {
  const m = option("model", "model", "sonnet", [choice("opus"), choice("sonnet")]);
  for (const saved of [undefined, ""]) {
    assert.equal(shownChoice(m, saved), "sonnet");
    assert.deepEqual(
      choiceList(m, saved).filter((c) => c.on).map((c) => c.id),
      ["sonnet"],
    );
  }
});

test("the agent's current model is marked now, saved or not", () => {
  const m = option("model", "model", "sonnet", [choice("opus"), choice("sonnet")]);
  const empty = choiceList(m, "");
  assert.deepEqual(empty.find((c) => c.id === "sonnet"), { ...choice("sonnet"), on: true, now: true, missing: false }, "nothing saved: now and picked");
  const saved = choiceList(m, "opus");
  assert.deepEqual(saved.map((c) => [c.id, c.on, c.now]), [["opus", true, false], ["sonnet", false, true]]);
});

test("a saved model shows as picked; one the agent no longer lists shows as it is, marked", () => {
  const m = option("model", "model", "sonnet", [choice("opus"), choice("sonnet")]);
  assert.equal(shownChoice(m, "opus"), "opus");
  // What a run sends is the saved id: the list shows that, not the current one.
  assert.equal(shownChoice(m, "gone"), "gone");
  assert.equal(missingChoice(m, "gone"), "gone");
  assert.equal(missingChoice(m, "opus"), null);
  assert.equal(missingChoice(m, undefined), null);
  const list = choiceList(m, "gone");
  assert.deepEqual(list.map((c) => [c.id, c.on, c.now, c.missing]), [["gone", true, false, true], ["opus", false, false, false], ["sonnet", false, true, false]]);
});

test("an agent whose current model is not among its choices still has one picked when nothing is saved", () => {
  const m = option("model", "model", "auto", [choice("opus"), choice("sonnet")]);
  assert.equal(shownChoice(m, undefined), "auto");
  assert.deepEqual(choiceList(m, undefined).filter((c) => c.on), [], "nothing listed is ticked: the run sends nothing, and the agent picks");
});
