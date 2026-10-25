/** Finding text in a conversation, without a browser: `node --test`. */

import { strict as assert } from "node:assert";
import { test } from "node:test";
import {
  collect,
  countLabel,
  decisionRegions,
  findKey,
  inReportDetails,
  jumpTo,
  keepIndex,
  markdownText,
  normalizeQuery,
  rangesIn,
  spots,
  startIndex,
  stepIndex,
  stepOf,
  turnRegions,
  type FindTurn,
  type Match,
  type Piece,
  type TurnRegions,
} from "./chatFind.ts";

const plain = { matchCase: false, wholeWord: false };

test("a query is composed, its spaces made one, and blank finds nothing", () => {
  assert.equal(normalizeQuery("  Hello   World "), " hello world ");
  assert.equal(normalizeQuery("   "), "");
  assert.equal(normalizeQuery(""), "");
  assert.equal(normalizeQuery("ABC", true), "ABC");
  // 한 typed as jamo (NFD) finds the composed syllable.
  assert.equal(normalizeQuery("한"), "한");
});

test("ranges are case-insensitive by default, in the text's own offsets", () => {
  assert.deepEqual(rangesIn("Parser parser PARSER", "parser", plain), [[0, 6], [7, 13], [14, 20]]);
  assert.deepEqual(rangesIn("Parser parser PARSER", "parser", { ...plain, matchCase: true }), [[7, 13]]);
});

test("ranges skip runs of white space, which match one space", () => {
  // "a  \n b" holds "a b": the range covers all of the space between.
  assert.deepEqual(rangesIn("x a  \n b y", "a b", plain), [[2, 8]]);
});

test("matches do not overlap", () => {
  assert.deepEqual(rangesIn("aaaa", "aa", plain), [[0, 2], [2, 4]]);
});

test("Korean and English mixed", () => {
  const text = "검색 기능을 추가했습니다. Search는 대소문자를 가리지 않습니다. 검색어";
  assert.equal(rangesIn(text, "검색", plain).length, 2);
  assert.equal(rangesIn(text, "search", plain).length, 1);
  assert.deepEqual(rangesIn(text, "기능을", plain), [[3, 6]]);
});

test("whole word wants a boundary where the query has a letter at its edge", () => {
  const w = { ...plain, wholeWord: true };
  assert.deepEqual(rangesIn("cat concat cat_x cat.", "cat", w), [[0, 3], [17, 20]]);
  // A Korean word with a particle on it is not the word alone.
  assert.equal(rangesIn("검색을 했다. 검색 끝", "검색", w).length, 1);
  // An edge that is not a letter needs nothing.
  assert.equal(rangesIn("a-b", "-b", w).length, 1);
  // Letters outside the BMP count as letters.
  assert.equal(rangesIn("\u{1D400}cat", "cat", w).length, 0);
});

test("a letter that would change length when lowered keeps its place", () => {
  // "İ" lowers to two code units; offsets after it must still be right.
  assert.deepEqual(rangesIn("İx abc", "abc", plain), [[3, 6]]);
});

test("markdown reads as the page shows it", () => {
  assert.equal(markdownText("Hello **wo`r`ld**").trim(), "Hello world");
  assert.equal(markdownText("[the docs](https://example.test/x)").trim(), "the docs");
  assert.equal(markdownText("a &amp; b").trim(), "a & b");
  assert.match(markdownText("```rust\nfn main() {}\n```"), /fn main\(\) \{\}/);
  assert.doesNotMatch(markdownText("```rust\nfn main() {}\n```"), /rust|```/);
  const table = markdownText("| a | b |\n|---|---|\n| 셀 | d |");
  assert.equal(rangesIn(table, "셀", plain).length, 1);
  // Two paragraphs do not run together into one word.
  assert.equal(rangesIn(markdownText("end\n\nstart"), "endstart", plain).length, 0);
  assert.equal(rangesIn(markdownText("line one\nline two"), "one line", plain).length, 1);
  assert.equal(markdownText("- [ ] task *one*").trim(), "task one");
});

const said = (turn: number, text: string): FindTurn => ({ turn, head: { kind: "me", text }, segments: null, message: "", error: "", decisions: [] });

test("matches come in reading order: turn by turn, region by region", () => {
  const turns: TurnRegions[] = [
    { turn: 0, regions: [{ key: "p", text: "find me" }, { key: "m", text: "me and me" }] },
    { turn: 1, regions: [{ key: "p", text: "nothing" }] },
    { turn: 2, regions: [{ key: "t0", text: "ME" }] },
  ];
  const got = collect(turns, "me", plain).map((m) => `${m.turn}:${m.key}:${m.k}`);
  assert.deepEqual(got, ["0:p:0", "0:m:0", "0:m:1", "2:t0:0"]);
  assert.deepEqual(collect(turns, " ", plain), []);
});

test("a turn's regions follow the timeline's order, steps only when asked", () => {
  const t: FindTurn = {
    turn: 3,
    head: { kind: "me", text: "please check" },
    segments: [
      { kind: "text", text: "Looking." },
      { kind: "thought", text: "  check the parser  " },
      { kind: "tool", tool: { id: "1", title: "Read src/parser.rs", toolKind: "read", status: "completed" } },
      { kind: "text", text: "Done: **check** passed." },
    ],
    message: "ignored once segments are known",
    error: "",
    decisions: [],
  };
  assert.deepEqual(
    turnRegions(t, false).map((r) => r.key),
    ["p", "t0", "t2"],
  );
  const withSteps = turnRegions(t, true);
  assert.deepEqual(
    withSteps.map((r) => r.key),
    ["p", "t0", "s0.0", "s0.1", "t2"],
  );
  // A step reads as the list shows it: the thought trimmed, the tool without its verb.
  assert.equal(withSteps[2].text, "check the parser");
  assert.equal(withSteps[3].text, "src/parser.rs");
  assert.equal(withSteps[4].text.trim(), "Done: check passed.");
});

test("a restored turn reads its folded message; hook chatter is not searched", () => {
  const t: FindTurn = { ...said(0, ""), message: "Notice: hook says: secret\nVisible *text*" };
  const r = turnRegions(t, false);
  assert.deepEqual(
    r.map((x) => x.key),
    ["m"],
  );
  assert.equal(collect([{ turn: 0, regions: r }], "secret", plain).length, 0);
  assert.equal(collect([{ turn: 0, regions: r }], "visible text", plain).length, 1);
});

test("a report turn searches the card, its lists included; a missing report nothing", () => {
  const report = {
    summary: "Parser fixed",
    questions: ["merge now?"],
    changes: [{ path: "src/a.rs", what: "parser fix" }],
    checks: [{ what: "cargo test", detail: "" }],
    risks: [],
    next: ["parser docs"],
  };
  const t: FindTurn = { turn: 0, head: { kind: "report", report }, segments: null, message: "noted", error: "", decisions: [] };
  const keys = collect([{ turn: 0, regions: turnRegions(t, false) }], "parser", plain).map((m) => m.key);
  assert.deepEqual(keys, ["r.s", "r.c0w", "r.n0"]);
  assert.ok(!inReportDetails("r.s") && !inReportDetails("r.q0"));
  assert.ok(inReportDetails("r.c0w") && inReportDetails("r.k1") && inReportDetails("r.n0"));
  assert.deepEqual(
    turnRegions({ ...t, head: { kind: "report", report: undefined } }, false).map((r) => r.key),
    ["m"],
  );
});

test("a worker's turn: the conductor's markdown, the answer, then the worker's own report", () => {
  const report = { summary: "done with **it**", questions: [], changes: [], checks: [], risks: [], next: [] };
  const t: FindTurn = { turn: 0, head: { kind: "markdown", text: "Fix the **parser**" }, segments: null, message: "parser ok", report, error: "parser failed", decisions: [] };
  const r = turnRegions(t, false);
  assert.deepEqual(
    r.map((x) => x.key),
    ["p", "m", "r.s", "e"],
  );
  assert.equal(r[0].text.trim(), "Fix the parser");
});

test("a decision card is searched as it is drawn in each state", () => {
  const base = {
    id: 7,
    question: "Which store?",
    context: "pick one",
    options: [{ label: "SQLite", detail: "local" }, { label: "Files" }],
    choice: 0,
    answer: null,
    note: "",
  };
  assert.deepEqual(
    decisionRegions({ ...base, status: "open" }).map((r) => r.key),
    ["d7.q", "d7.c", "d7.o0", "d7.o0d", "d7.o1"],
  );
  assert.deepEqual(
    decisionRegions({ ...base, status: "decided", note: "fine" }).map((r) => [r.key, r.text]),
    [
      ["d7.q", "Which store?"],
      ["d7.a", "SQLite"],
      ["d7.n", "fine"],
      ["d7.o1", "Files"],
    ],
  );
  assert.deepEqual(
    decisionRegions({ ...base, status: "decided", choice: null, answer: "my own" }).map((r) => r.text),
    ["Which store?", "my own", "SQLite", "Files"],
  );
  assert.deepEqual(
    decisionRegions({ ...base, status: "dismissed" }).map((r) => r.key),
    ["d7.q"],
  );
});

test("keys name what the page draws", () => {
  assert.equal(findKey.step(2, 5), "s2.5");
  assert.deepEqual(stepOf("s2.5"), { at: 2, i: 5 });
  assert.equal(stepOf("t2"), null);
  assert.equal(findKey.decision.optionDetail(3, 1), "d3.o1d");
});

const m = (turn: number, key = "p", k = 0): Match => ({ turn, key, k });

test("a new search starts at the nearest match at or above the turn in view", () => {
  const all = [m(1), m(4), m(4, "m", 1), m(9)];
  assert.equal(startIndex(all, 100), 3, "at the foot: the newest");
  assert.equal(startIndex(all, 5), 2, "the last one above");
  assert.equal(startIndex(all, 4), 2);
  assert.equal(startIndex(all, 0), 0, "below them all: the first");
  assert.equal(startIndex([], 3), -1);
});

test("next and previous go round", () => {
  assert.equal(stepIndex(-1, 3, false), 0);
  assert.equal(stepIndex(-1, 3, true), 2);
  assert.equal(stepIndex(2, 3, false), 0);
  assert.equal(stepIndex(0, 3, true), 2);
  assert.equal(stepIndex(1, 3, true), 0);
  assert.equal(stepIndex(0, 0, false), -1);
});

test("the current match keeps its place when the matches change", () => {
  const order = (x: Match) => x.turn;
  const before = m(4, "m", 1);
  assert.equal(keepIndex(before, [m(1), m(4), m(4, "m", 1)], order), 2, "still there");
  assert.equal(keepIndex(before, [m(1), m(5), m(9)], order), 1, "gone: the next one after");
  assert.equal(keepIndex(before, [m(1), m(2)], order), 1, "nothing after: the last");
  assert.equal(keepIndex(undefined, [m(1)], order), -1);
  assert.equal(keepIndex(before, [], order), -1);
});

test("a match outside the drawn page is revealed; the decisions under it all are always drawn", () => {
  assert.equal(jumpTo(25, 20, 40, 40), "drawn");
  assert.equal(jumpTo(19, 20, 40, 40), "reveal");
  assert.equal(jumpTo(40, 20, 30, 40), "drawn", "orphan decisions sit under every turn");
  assert.equal(jumpTo(35, 20, 30, 40), "reveal", "below a bounded window");
});

test("the count reads i/n", () => {
  assert.deepEqual(countLabel(2, 17), { i: 3, n: 17 });
  assert.deepEqual(countLabel(-1, 4), { i: 1, n: 4 });
  assert.equal(countLabel(0, 0), null);
});

test("spots map a match onto text nodes, across inline marks and around gaps", () => {
  // <p>Hello <b>wor</b>ld</p><p>next</p> as pieces.
  const pieces: Piece<string>[] = [
    { node: null, text: " " },
    { node: "a", text: "Hello " },
    { node: "b", text: "wor" },
    { node: "c", text: "ld" },
    { node: null, text: " " },
    { node: null, text: " " },
    { node: "d", text: "next" },
    { node: null, text: " " },
  ];
  assert.deepEqual(spots(pieces, "world", plain), [{ start: 2, startAt: 0, end: 3, endAt: 2 }]);
  assert.deepEqual(spots(pieces, "lo w", plain), [{ start: 1, startAt: 3, end: 2, endAt: 1 }]);
  // Across the paragraph gap: starts on "ld", ends on "next".
  assert.deepEqual(spots(pieces, "ld next", plain), [{ start: 3, startAt: 0, end: 6, endAt: 4 }]);
  // A query with a leading space starting on the gap moves onto the text.
  assert.deepEqual(spots(pieces, " next", plain), [{ start: 6, startAt: 0, end: 6, endAt: 4 }]);
  assert.deepEqual(spots(pieces, "zzz", plain), []);
});

test("collecting a long history is quick", () => {
  const turns: TurnRegions[] = [];
  for (let i = 0; i < 800; i++) {
    const t = turnRegions(
      {
        turn: i,
        head: { kind: "me", text: `질문 ${i}: parser 상태는 어떤가요?` },
        segments: null,
        message: `## 답 ${i}\n\nThe **parser** passes. 검색 결과는 ${"lorem ipsum ".repeat(60)}\n\n- one\n- two`,
        error: "",
        decisions: [],
      },
      false,
    );
    turns.push({ turn: i, regions: t });
  }
  collect(turns, "parser", plain);
  const t0 = performance.now();
  const n = collect(turns, "parser", plain).length;
  const ms = performance.now() - t0;
  assert.equal(n, 1600);
  assert.ok(ms < 100, `${ms.toFixed(1)} ms`);
});
