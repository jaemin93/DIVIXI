/** Sending a failed conductor turn again: `node --test`. */

import { strict as assert } from "node:assert";
import { test } from "node:test";
import { againState, asSent, offersSendAgain, sentAgainAs, type Turn } from "./sendAgain.ts";

const turn = (id: string, over: Partial<Turn> = {}): Turn => ({ id, session: "conductor", status: "failed", prompt: "look at the build", startedAt: 1000, ...over });
const PERMISSION = "[worker-permission]";

test("a conductor turn that failed is offered; one that went through, was stopped or is a worker's is not", () => {
  assert.equal(offersSendAgain(turn("t1"), [PERMISSION]), true);
  assert.equal(offersSendAgain(turn("t1", { prompt: "[schedule] #26 · 11-06 05:00 예정\n정리" }), [PERMISSION]), true, "a schedule's turn");
  assert.equal(offersSendAgain(turn("t1", { status: "done" }), [PERMISSION]), false, "a stop by the human ends a turn as done");
  assert.equal(offersSendAgain(turn("t1", { status: "interrupted" }), [PERMISSION]), false, "the app's cut: the comeback says it");
  assert.equal(offersSendAgain(turn("t1", { status: "running" }), [PERMISSION]), false);
  assert.equal(offersSendAgain(turn("t1", { session: "ui" }), [PERMISSION]), false, "a worker's: the conductor asks it again");
  assert.equal(offersSendAgain(turn("t1", { prompt: `${PERMISSION} worker=ui run=t3\nwants to: rm -rf build` }), [PERMISSION]), false, "nobody waits on that answer now");
  assert.equal(offersSendAgain(turn("t1", { prompt: "  " }), [PERMISSION]), false);
});

test("a stored prompt splits back into what was sent: the files only from the list the core appends last", () => {
  assert.deepEqual(asSent("look\n\n[attachments]\n- /w/a.png\n- /w/b c.txt"), { text: "look", files: ["/w/a.png", "/w/b c.txt"] });
  assert.deepEqual(asSent("\n\n[attachments]\n- /w/a.png"), { text: "", files: ["/w/a.png"] }, "files and no words");
  assert.deepEqual(asSent("look"), { text: "look", files: [] });
  const quoted = "[worker-report] worker=ui run=t3 status=done\n\nthe log said:\n\n[attachments]\n- /w/a.png\nand then it stopped";
  assert.deepEqual(asSent(quoted), { text: quoted, files: [] }, "the mark quoted inside, with words after: not a list");
  const both = "see\n\n[attachments]\nnot a list\n\n[attachments]\n- /w/a.png";
  assert.deepEqual(asSent(both), { text: "see\n\n[attachments]\nnot a list", files: ["/w/a.png"] }, "the last list is the core's");
});

test("it went again when a later turn carried the same words; waiting or held when the line has it", () => {
  const failed = turn("t1");
  const conversation = [failed, turn("t2", { prompt: "something else", startedAt: 2000, status: "done" })];
  assert.equal(againState(failed, conversation, []), null);
  const again = turn("t3", { startedAt: 3000, status: "connecting" });
  assert.equal(sentAgainAs(failed, [...conversation, again])?.id, "t3", "typed, or the button's turn on its way");
  assert.equal(againState(failed, [...conversation, again], []), "sent");
  assert.equal(sentAgainAs(failed, [turn("t0", { startedAt: 500 }), failed]), undefined, "an earlier turn with the same words is not it");
  assert.equal(sentAgainAs(failed, [failed, turn("t4", { session: "ui", startedAt: 3000 })]), undefined, "nor a worker's");
  assert.equal(againState(failed, conversation, [{ again: "t1" }]), "waiting", "in the line for the turn in flight");
  assert.equal(againState(failed, conversation, [{ again: "t1", held: true }]), "held", "read back at startup, waiting on the human");
  assert.equal(againState(failed, conversation, [{ again: "t9" }, {}]), null);
});
