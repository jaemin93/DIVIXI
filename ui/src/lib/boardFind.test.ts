import test from "node:test";
import assert from "node:assert/strict";
import { findCards, fold, isFindKey, stepMatch } from "./boardFind.ts";

const card = (id: string, kind: string, x: number, y: number, text: string, more: Record<string, string> = {}) => ({ id, kind, x, y, text, ...more });
const board = [
  card("n12", "note", 300, 0, "양쪽 곳 비교: 왼쪽과 오른쪽"),
  card("n3", "note", 0, 0, "결정문이 문을 열어 두면"),
  card("n441", "note", 0, 400, "비교 기준을 정한다"),
  card("n44", "question", 0, 200, "무엇과 비교하나?", { answer: "경쟁 제품 셋" }),
  card("n7", "frame", 600, 300, "Release notes"),
  card("n8", "link", 900, 0, "", { name: "Design system", url: "https://example.dev/ds" }),
  card("n9", "sketch", 0, 900, "비교"),
];

test("a card is found by its words, in Korean too, top to bottom then left to right", () => {
  assert.deepEqual(findCards(board, "비교"), ["n12", "n44", "n441"], "a sketch says nothing");
  assert.deepEqual(findCards(board, "  문을   열어 "), ["n3"], "spaces as one");
  assert.deepEqual(findCards(board, "경쟁"), ["n44"], "a question's answer");
  assert.deepEqual(findCards(board, "release"), ["n7"], "a frame's title, case aside");
  assert.deepEqual(findCards(board, "design SYSTEM"), ["n8"], "a link's name");
  assert.deepEqual(findCards(board, "example.dev"), ["n8"], "and its address");
  assert.deepEqual(findCards(board, ""), []);
  assert.deepEqual(findCards(board, "없는 말"), []);
});

test("a Korean query typed in jamo finds the syllables", () => {
  const decomposed = "비교".normalize("NFD");
  assert.notEqual(decomposed, "비교");
  assert.deepEqual(findCards(board, decomposed), ["n12", "n44", "n441"]);
  assert.equal(fold("Ａ  B"), "ａ b".normalize("NFC").toLocaleLowerCase());
});

test("an id the agent named comes first, exactly, any case", () => {
  assert.deepEqual(findCards(board, "n441"), ["n441"], "n441, not n44 or n4…");
  assert.deepEqual(findCards(board, "N44"), ["n44"]);
  // A word that is also an id: the card with that id first, then the cards that say it.
  const both = [...board, card("n50", "note", 0, 50, "see n3 above")];
  assert.deepEqual(findCards(both, "n3"), ["n3", "n50"]);
});

test("Enter steps forward, Shift+Enter back, round the ends", () => {
  assert.equal(stepMatch(-1, 3, false), 0);
  assert.equal(stepMatch(-1, 3, true), 2);
  assert.equal(stepMatch(2, 3, false), 0);
  assert.equal(stepMatch(0, 3, true), 2);
  assert.equal(stepMatch(1, 3, false), 2);
  assert.equal(stepMatch(0, 0, false), -1, "nothing to step through");
});

test("the find key is Ctrl+F, Cmd+F on macOS, and not with Alt", () => {
  const k = (key: string, mods: Partial<{ ctrlKey: boolean; metaKey: boolean; altKey: boolean }> = {}) => ({ key, ctrlKey: false, metaKey: false, altKey: false, ...mods });
  assert.ok(isFindKey(k("f", { ctrlKey: true }), false));
  assert.ok(isFindKey(k("F", { ctrlKey: true }), false), "with Shift too");
  assert.ok(!isFindKey(k("f", { metaKey: true }), false));
  assert.ok(isFindKey(k("f", { metaKey: true }), true));
  assert.ok(!isFindKey(k("f", { ctrlKey: true }), true), "Ctrl+F on a Mac is the terminal's");
  assert.ok(!isFindKey(k("f", { ctrlKey: true, altKey: true }), false));
  assert.ok(!isFindKey(k("g", { ctrlKey: true }), false));
});
