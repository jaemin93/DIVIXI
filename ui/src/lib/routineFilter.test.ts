import test from "node:test";
import assert from "node:assert/strict";
import { ROUTINE_TAGS_KEY, routinesShown, routineTagCount } from "./routineFilter.ts";
import { toggleTag } from "./trackFilter.ts";
import { LAYOUT_SETTINGS } from "./layoutSettings.ts";

const routines = [
  { name: "Dependency check", instruction: "list stale deps", tags: ["ops", "weekly"] },
  { name: "Log sweep", instruction: "read the error log", tags: ["ops"] },
  { name: "Release notes", instruction: "draft notes", tags: ["weekly"] },
  { name: "Untagged", instruction: "anything" },
];
const names = (list: { name: string }[]) => list.map((r) => r.name);

test("picked tags are AND-ed: a routine shows only when it carries every one", () => {
  assert.deepEqual(names(routinesShown(routines, "", ["ops"])), ["Dependency check", "Log sweep"]);
  assert.deepEqual(names(routinesShown(routines, "", ["ops", "weekly"])), ["Dependency check"]);
  assert.deepEqual(names(routinesShown(routines, "", ["ops", "none"])), [], "a filter that leaves nothing leaves an empty list");
});

test("clearing the tags shows every routine again", () => {
  let picked = toggleTag([], "ops");
  picked = toggleTag(picked, "weekly");
  assert.equal(routinesShown(routines, "", picked).length, 1);
  picked = [];
  assert.equal(routinesShown(routines, "", picked).length, routines.length);
  assert.deepEqual(toggleTag(["ops"], "ops"), [], "picking a picked tag again takes it off");
});

test("the search narrows by name, instruction or tag, with the tags", () => {
  assert.deepEqual(names(routinesShown(routines, "LOG", [])), ["Log sweep"]);
  assert.deepEqual(names(routinesShown(routines, "draft", [])), ["Release notes"]);
  assert.deepEqual(names(routinesShown(routines, "week", ["ops"])), ["Dependency check"]);
});

test("the menu counts the routines carrying each tag", () => {
  assert.equal(routineTagCount(routines, "ops"), 2);
  assert.equal(routineTagCount(routines, "none"), 0);
});

test("the routines' filter is kept under its own setting, as layout", () => {
  assert.equal(ROUTINE_TAGS_KEY, "routinelist_tags");
  for (const other of ["tracks_filter", "designlist_tags", "kblist_tags"]) assert.notEqual(ROUTINE_TAGS_KEY, other);
  assert.ok((LAYOUT_SETTINGS as readonly string[]).includes(ROUTINE_TAGS_KEY), "kept on this PC only, as the other columns' filters");
});
