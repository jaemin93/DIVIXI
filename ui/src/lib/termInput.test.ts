import test from "node:test";
import assert from "node:assert/strict";
import { CompositionGuard, InputQueue } from "./termInput.ts";

/** A send whose answers come back when the test says, in any order. */
function slowSend() {
  const sent: string[] = [];
  const answers: (() => void)[] = [];
  const send = (data: string) =>
    new Promise<void>((resolve) => {
      sent.push(data);
      answers.push(resolve);
    });
  return { sent, answers, send };
}

const tick = () => new Promise((r) => setTimeout(r, 0));

test("the first key goes at once", () => {
  const { sent, send } = slowSend();
  const q = new InputQueue(send);
  q.push("a");
  assert.deepEqual(sent, ["a"]);
});

test("nothing is sent while the one before is on its way, and what waited goes together, in order", async () => {
  const { sent, answers, send } = slowSend();
  const q = new InputQueue(send);
  // A Korean IME: the last syllable is committed, "." follows at once.
  q.push("다");
  q.push(".");
  q.push("\r");
  assert.deepEqual(sent, ["다"]);
  answers[0]();
  await tick();
  assert.deepEqual(sent, ["다", ".\r"]);
  answers[1]();
  await q.idle();
  assert.equal(sent.join(""), "다.\r");
});

test("a whole sentence typed against a slow link arrives exactly", async () => {
  const received: string[] = [];
  // Each request answers after a random delay; without the queue the
  // answers' order would be the order written.
  const q = new InputQueue(
    (data) =>
      new Promise<void>((resolve) =>
        setTimeout(() => {
          received.push(data);
          resolve();
        }, Math.random() * 5),
      ),
  );
  for (const piece of ["대", "단", "히", " ", "감", "사", "합", "니", "다", ".", "\r"]) q.push(piece);
  await q.idle();
  assert.equal(received.join(""), "대단히 감사합니다.\r");
  assert.ok(received.length <= 3, `sent in ${received.length} requests`);
});

test("a failed send does not stop the ones after it", async () => {
  const sent: string[] = [];
  let fail = true;
  const q = new InputQueue(async (data) => {
    sent.push(data);
    if (fail) {
      fail = false;
      throw new Error("gone");
    }
  });
  q.push("a");
  q.push("b");
  await q.idle();
  assert.deepEqual(sent, ["a", "b"]);
  q.push("c");
  await q.idle();
  assert.deepEqual(sent, ["a", "b", "c"]);
});

test("empty input sends nothing", async () => {
  const { sent, send } = slowSend();
  const q = new InputQueue(send);
  q.push("");
  await q.idle();
  assert.deepEqual(sent, []);
});

test("paste, arrows, Ctrl+C and Backspace keep their places among typed text", async () => {
  const { sent, answers, send } = slowSend();
  const q = new InputQueue(send);
  q.push("\x1b[200~pasted\x1b[201~");
  q.push("\x1b[D");
  q.push("\x7f");
  q.push("\x03");
  answers[0]();
  await tick();
  answers[1]();
  await q.idle();
  assert.deepEqual(sent, ["\x1b[200~pasted\x1b[201~", "\x1b[D\x7f\x03"]);
});

/** A guard whose timers the test runs. */
function guard() {
  const timers: (() => void)[] = [];
  const g = new CompositionGuard((fn) => timers.push(fn));
  return { g, timers };
}

const keydown = { type: "keydown" };
const keypress = { type: "keypress" };

test("keys pass when nothing is being composed", () => {
  const { g } = guard();
  assert.ok(g.allows(keydown));
  assert.ok(g.allows(keypress));
});

test("a keypress right after a composition ends is left to the textarea", () => {
  const { g } = guard();
  g.ended();
  assert.equal(g.allows(keypress), false);
  // A keydown (Enter, Backspace, an arrow) is xterm's: it sends the
  // composed text itself, at once, before the key.
  assert.ok(g.allows(keydown));
});

test("once xterm has sent the composed text, keypresses are its own again", () => {
  const { g } = guard();
  g.ended();
  g.sent();
  assert.ok(g.allows(keypress));
});

test("a composition that sent nothing frees keypresses after its timer", () => {
  const { g, timers } = guard();
  g.ended();
  assert.equal(g.allows(keypress), false);
  timers.forEach((fn) => fn());
  assert.ok(g.allows(keypress));
});
