/** The error list and how long an error lives: `node --test`. */

import { strict as assert } from "node:assert";
import { test } from "node:test";
import { addError, dropError, errorLife, type AppError } from "./errors.ts";

const texts = (list: AppError[]) => list.map((e) => e.text);

test("an error joins the list, newest last", () => {
  let list: AppError[] = [];
  list = addError(list, "could not open the folder", 1, 1000);
  list = addError(list, "the agent is not installed", 2, 2000);
  assert.deepEqual(texts(list), ["could not open the folder", "the agent is not installed"]);
  assert.deepEqual(
    list.map((e) => e.id),
    [1, 2],
  );
});

test("nothing is raised for empty or blank text", () => {
  assert.deepEqual(addError([], "", 1, 0), []);
  assert.deepEqual(addError([], "   \n ", 1, 0), []);
});

test("text is trimmed on the way in", () => {
  assert.equal(addError([], "  spaced out  ", 1, 0)[0].text, "spaced out");
});

test("the same failure twice running is counted, not stacked", () => {
  // A retry loop must not bury the screen in copies of one message.
  let list: AppError[] = [];
  list = addError(list, "connection refused", 1, 1000);
  list = addError(list, "connection refused", 2, 5000);
  list = addError(list, "connection refused", 3, 9000);
  assert.equal(list.length, 1);
  assert.equal(list[0].again, 2, "three times over is one error, twice again");
  assert.equal(list[0].id, 1, "it stays the same error, so its toast does not jump");
  assert.equal(list[0].at, 9000, "and its clock starts again from the latest");
});

test("a different failure in between keeps them apart", () => {
  let list: AppError[] = [];
  list = addError(list, "one", 1, 0);
  list = addError(list, "two", 2, 0);
  list = addError(list, "one", 3, 0);
  assert.deepEqual(texts(list), ["one", "two", "one"]);
  assert.ok(list.every((e) => e.again === 0));
});

test("dismissing takes only that one out", () => {
  let list: AppError[] = [];
  list = addError(list, "one", 1, 0);
  list = addError(list, "two", 2, 0);
  list = dropError(list, 1);
  assert.deepEqual(texts(list), ["two"]);
  // An id that is not there changes nothing.
  assert.deepEqual(texts(dropError(list, 99)), ["two"]);
});

test("a short error lives the floor, a long one longer, none forever", () => {
  assert.equal(errorLife("no"), 6_000);
  assert.equal(errorLife("x".repeat(60)), 6_000, "the allowance starts past sixty characters");
  assert.equal(errorLife("x".repeat(80)), 6_000 + 20 * 45);
  assert.equal(errorLife("x".repeat(10_000)), 20_000, "capped, however long the message");
  // Always long enough to read something, never long enough to be furniture.
  for (const n of [0, 1, 50, 100, 500, 5000]) {
    const life = errorLife("x".repeat(n));
    assert.ok(life >= 6_000 && life <= 20_000, `${n} characters gave ${life}ms`);
  }
});
