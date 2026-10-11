import test from "node:test";
import assert from "node:assert/strict";
import { pickable } from "./agentOptions.ts";

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
