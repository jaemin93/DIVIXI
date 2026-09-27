/**
 * The waiting line, without a running app: `node --test`.
 *
 * Node 24 strips the types, so this runs straight from source.
 */

import { strict as assert } from "node:assert";
import { test } from "node:test";
import {
  dropQueued,
  liveQueued,
  liveQueuedFor,
  nextQueued,
  parseQueued,
  pushQueued,
  queuedFor,
  queuedKeys,
  releaseQueued,
  unshiftQueued,
  wasStopped,
  type Queued,
} from "./queue.ts";

function q(id: string, key = "track:a", extra: Partial<Queued> = {}): Queued {
  return {
    id,
    key,
    target: key.slice(key.indexOf(":") + 1),
    agent: "claude_code",
    text: id,
    files: [],
    picks: [],
    selected: [],
    at: 0,
    ...extra,
  };
}

const ids = (list: Queued[]) => list.map((x) => x.id);

test("waiting messages go out in the order they were sent", () => {
  let list: Queued[] = [];
  for (const id of ["one", "two", "three"]) list = pushQueued(list, q(id));
  const out: string[] = [];
  for (;;) {
    const { next, rest } = nextQueued(list, "track:a");
    if (!next) break;
    out.push(next.id);
    list = rest;
  }
  assert.deepEqual(out, ["one", "two", "three"]);
});

test("a conversation only sees its own line", () => {
  let list: Queued[] = [];
  list = pushQueued(list, q("a1", "track:a"));
  list = pushQueued(list, q("b1", "artifact:b"));
  list = pushQueued(list, q("a2", "track:a"));
  assert.deepEqual(ids(queuedFor(list, "track:a")), ["a1", "a2"]);
  assert.deepEqual(ids(queuedFor(list, "artifact:b")), ["b1"]);
  assert.deepEqual(queuedKeys(list), ["track:a", "artifact:b"]);
  // Taking one track's turn leaves the other's alone.
  const { next, rest } = nextQueued(list, "track:a");
  assert.equal(next?.id, "a1");
  assert.deepEqual(ids(queuedFor(rest, "artifact:b")), ["b1"]);
});

test("a send that could not start puts the message back at the front of its line", () => {
  let list: Queued[] = [];
  list = pushQueued(list, q("b1", "artifact:b"));
  list = pushQueued(list, q("a1", "track:a"));
  list = pushQueued(list, q("a2", "track:a"));
  const { next, rest } = nextQueued(list, "track:a");
  assert.equal(next?.id, "a1");
  const back = unshiftQueued(rest, next!);
  assert.deepEqual(ids(queuedFor(back, "track:a")), ["a1", "a2"]);
  // The other conversation keeps its place.
  assert.deepEqual(ids(queuedFor(back, "artifact:b")), ["b1"]);
});

test("putting back into an empty line still works", () => {
  assert.deepEqual(ids(unshiftQueued([], q("only"))), ["only"]);
});

test("cancelling takes one message out and leaves the rest in order", () => {
  let list: Queued[] = [];
  for (const id of ["one", "two", "three"]) list = pushQueued(list, q(id));
  list = dropQueued(list, "two");
  assert.deepEqual(ids(queuedFor(list, "track:a")), ["one", "three"]);
  // An id that is not there changes nothing.
  assert.equal(dropQueued(list, "gone").length, 2);
});

test("stopping a turn does not empty the line: it is the next turn's", () => {
  // The store never touches the line on a cancel; the run simply ends as
  // stopped and the line is pumped again. This is that invariant in small.
  let list: Queued[] = [];
  list = pushQueued(list, q("one"));
  list = pushQueued(list, q("two"));
  assert.equal(wasStopped({ status: "done", stopReason: "Cancelled" }), true);
  assert.deepEqual(ids(queuedFor(list, "track:a")), ["one", "two"]);
  assert.equal(nextQueued(list, "track:a").next?.id, "one");
});

test("a turn that ended on its own, failed, or is still running is not stopped", () => {
  assert.equal(wasStopped({ status: "done", stopReason: "end_turn" }), false);
  assert.equal(wasStopped({ status: "done" }), false);
  assert.equal(wasStopped({ status: "failed", stopReason: "cancelled" }), false);
  assert.equal(wasStopped({ status: "running", stopReason: "cancelled" }), false);
});

// ----- messages that came back with the app -----

test("a held message is shown but never sent by itself", () => {
  let list: Queued[] = [];
  list = pushQueued(list, q("old", "track:a", { held: true }));
  // It is in the conversation…
  assert.deepEqual(ids(queuedFor(list, "track:a")), ["old"]);
  // …but nothing will pick it up, and the pump has no conversation to work.
  assert.equal(nextQueued(list, "track:a").next, undefined);
  assert.deepEqual(ids(liveQueued(list)), []);
  assert.deepEqual(queuedKeys(list), []);
});

test("a held message does not hold up one sent now", () => {
  let list: Queued[] = [];
  list = pushQueued(list, q("old", "track:a", { held: true }));
  list = pushQueued(list, q("new", "track:a"));
  assert.deepEqual(ids(liveQueuedFor(list, "track:a")), ["new"]);
  const { next, rest } = nextQueued(list, "track:a");
  assert.equal(next?.id, "new");
  // The held one is still there, waiting on the human.
  assert.deepEqual(ids(queuedFor(rest, "track:a")), ["old"]);
});

test("releasing a held message puts it in the line where it stands", () => {
  let list: Queued[] = [];
  list = pushQueued(list, q("old1", "track:a", { held: true }));
  list = pushQueued(list, q("old2", "track:a", { held: true }));
  list = releaseQueued(list, "old2");
  assert.deepEqual(ids(liveQueuedFor(list, "track:a")), ["old2"]);
  assert.equal(nextQueued(list, "track:a").next?.id, "old2");
  // Releasing an id that is not there changes nothing.
  assert.deepEqual(ids(releaseQueued(list, "gone")), ["old1", "old2"]);
});

test("a retry goes in front of the live line but behind held messages", () => {
  let list: Queued[] = [];
  list = pushQueued(list, q("old", "track:a", { held: true }));
  list = pushQueued(list, q("a1", "track:a"));
  list = pushQueued(list, q("a2", "track:a"));
  const { next, rest } = nextQueued(list, "track:a");
  const back = unshiftQueued(rest, next!);
  assert.deepEqual(ids(queuedFor(back, "track:a")), ["old", "a1", "a2"]);
});

test("the line read back from the setting comes back held, in order", () => {
  const saved = JSON.stringify([q("one"), q("two", "artifact:b", { at: 5 })]);
  const back = parseQueued(saved);
  assert.deepEqual(ids(back), ["one", "two"]);
  assert.ok(back.every((x) => x.held === true));
  assert.equal(back[1].key, "artifact:b");
  assert.equal(back[1].at, 5);
});

test("a setting that is missing, empty or not a line reads as nothing", () => {
  for (const raw of [null, undefined, "", "not json", '"a string"', "42", "{}"]) {
    assert.deepEqual(parseQueued(raw as string | null), []);
  }
});

test("entries that are not messages are dropped rather than guessed at", () => {
  const saved = JSON.stringify([
    q("good"),
    { id: "no-key" },
    null,
    "nope",
    { ...q("empty"), text: "  ", files: [], picks: [], selected: [] },
    { ...q("odd"), files: ["ok.txt", 7], agent: 3, at: "soon" },
  ]);
  const back = parseQueued(saved);
  assert.deepEqual(ids(back), ["good", "odd"]);
  const odd = back[1];
  assert.deepEqual(odd.files, ["ok.txt"]);
  assert.equal(odd.agent, "");
  assert.equal(typeof odd.at, "number");
});
