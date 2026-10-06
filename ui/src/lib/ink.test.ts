import test from "node:test";
import assert from "node:assert/strict";
import { briefOf } from "./ink.ts";
import type { DesignDoc } from "./store.svelte";

/** A board as the core sends it, with the words that once came out wrong, emoji, a combining mark and CJK. */
const words = {
  goal: "양쪽 곳 비교: 왼쪽과 오른쪽을 나란히",
  note: "결정문이 문을 열어 두면 되돌릴 수 있다",
  rare: "졸업생·졸음·뷁·똠얌꿍·흙탕물?",
  mixed: "이모지 🎨🧭 👩🏽‍💻 그리고 결합 문자 é, 한자 漢字",
};

const node = (id: string, text: string, tag = "") => ({ id, kind: "note", x: 0, y: 0, w: 260, h: 150, text, tag, by: "agent" });

const doc = {
  nodes: [node("n1", words.goal, "goal"), node("n2", words.note), node("n3", words.rare, "question"), node("n4", words.mixed, "idea")],
  edges: [],
  changes: [],
  version: 1,
} as unknown as DesignDoc;

test("the design's brief carries the board's words as they are", () => {
  // The board as JSON on the wire (the core's design_doc), then the brief made from it.
  const wire = JSON.parse(JSON.stringify(doc)) as DesignDoc;
  const brief = briefOf("한글 보드", wire, { goal: "목표", idea: "아이디어", question: "질문", note: "메모" });
  for (const w of Object.values(words)) assert.ok(brief.includes(w), `${w} in the brief`);
  assert.ok(brief.includes("é") && !brief.includes("é"), "the combining mark stays a combining mark");
  assert.ok(!/\\u[0-9a-fA-F]{4}/.test(brief), "no escapes");
});

test("JSON on the wire keeps the characters, not escapes", () => {
  const wire = JSON.stringify(doc);
  for (const w of Object.values(words)) assert.ok(wire.includes(w), w);
  assert.ok(!wire.includes("\\u"), "JSON.stringify writes Korean and emoji as themselves");
});
