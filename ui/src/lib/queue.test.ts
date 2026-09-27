/**
 * The waiting line, without a running app: `node --test`.
 *
 * Node 24 strips the types, so this runs straight from source.
 */

import { strict as assert } from "node:assert";
import { test } from "node:test";
import type { KbPick } from "./store.svelte.ts";
import {
  dropQueued,
  liveQueued,
  liveQueuedFor,
  nextQueued,
  notNow,
  parseQueued,
  pushQueued,
  queuedFor,
  queuedKeys,
  releaseQueued,
  stillWaiting,
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

// ----- what the second review found -----

/** A passage as the knowledge search really hands them over. */
function pick(id: number): KbPick {
  return {
    id,
    title: `p${id}`,
    source: "docs/a.md",
    section: null,
    line_start: 1,
    line_end: 9,
    summary: "",
    content: "the passage",
    tokens: 12,
    match_type: "fts",
  };
}

test("a saved passage that is not a passage is dropped, not cast", () => {
  // H3, from review-1/queue_probe.ts: `picks` used to be waved through on
  // being an array. Half a passage reaches `withKnowledge`, which reads
  // `.content.trim()`, and the TypeError costs the human the message.
  const raw = JSON.stringify([
    { id: "a", key: "track:tr1", target: "tr1", text: "hi", picks: [{ nope: 1 }, "a string", null] },
  ]);
  const back = parseQueued(raw);
  assert.equal(back.length, 1, "the message itself is still worth keeping");
  assert.deepEqual(back[0].picks, [], "but nothing it cannot read comes with it");
  // The access that used to throw.
  assert.doesNotThrow(() => back[0].picks.map((p) => p.content.trim()));
});

test("a whole passage survives the round trip, a maimed one does not", () => {
  const whole = pick(7);
  const { content: _gone, ...noContent } = pick(8);
  const raw = JSON.stringify([{ ...q("a"), picks: [whole, noContent, { ...pick(9), id: "9" }] }]);
  const back = parseQueued(raw);
  assert.deepEqual(
    back[0].picks.map((p) => p.id),
    [7],
  );
  assert.deepEqual(back[0].picks[0], whole);
});

test("passage ids come back as numbers, so the chips cannot collide on a key", () => {
  // Svelte throws on a duplicate key in release builds too, and it would
  // take the whole conversation's rendering with it.
  const raw = JSON.stringify([{ ...q("a"), picks: [pick(1), { nope: 1 }, { also: 2 }] }]);
  const ids = parseQueued(raw)[0].picks.map((p) => p.id);
  assert.deepEqual(ids, [1]);
  assert.equal(new Set(ids).size, ids.length);
});

test("a refused send puts the message back where it was", () => {
  // H2: the line is the only copy once `compose` has emptied the box.
  let list: Queued[] = [];
  list = pushQueued(list, q("a1"));
  list = pushQueued(list, q("a2"));
  const { next, rest } = nextQueued(list, "track:a");
  // …the send is refused, or throws…
  const back = unshiftQueued(rest, next!);
  assert.deepEqual(ids(queuedFor(back, "track:a")), ["a1", "a2"], "nothing lost, order kept");
});

test("a message held after a failure waits on the human instead of retrying", () => {
  let list: Queued[] = [];
  list = pushQueued(list, q("a1"));
  const { next, rest } = nextQueued(list, "track:a");
  const back = unshiftQueued(rest, { ...next!, held: true });
  assert.deepEqual(ids(queuedFor(back, "track:a")), ["a1"], "still in the conversation");
  assert.equal(nextQueued(back, "track:a").next, undefined, "but nothing sends it again by itself");
});

test("a message with a turn on screen has stopped waiting", () => {
  // The duplicate the human saw: the waiting bubble and the sent message
  // both on screen. A send refused as "still responding" puts the message
  // back in the line, and if an event had already claimed the run we made
  // for it, the run stays too. Whatever the cause, one of them has to go.
  let list: Queued[] = [];
  list = pushQueued(list, q("a1"));
  list = pushQueued(list, q("a2"));
  assert.deepEqual(ids(stillWaiting(list, "track:a", [])), ["a1", "a2"]);
  assert.deepEqual(ids(stillWaiting(list, "track:a", ["a1"])), ["a2"], "a1 is on screen as a turn now");
  assert.deepEqual(ids(stillWaiting(list, "track:a", ["a1", "a2"])), []);
});

test("another conversation's sent messages do not hide this one's", () => {
  let list: Queued[] = [];
  list = pushQueued(list, q("a1", "track:a"));
  list = pushQueued(list, q("b1", "artifact:b"));
  assert.deepEqual(ids(stillWaiting(list, "track:a", ["b1"])), ["a1"]);
  assert.deepEqual(ids(stillWaiting(list, "artifact:b", ["a1"])), ["b1"]);
});

test("a held message is hidden too once its turn exists", () => {
  const list = [q("old", "track:a", { held: true })];
  assert.deepEqual(ids(stillWaiting(list, "track:a", [])), ["old"]);
  assert.deepEqual(ids(stillWaiting(list, "track:a", ["old"])), []);
});

test('"not now" is the busy: prefix, not the words after it', () => {
  // M6. Both refusals the queue can meet carry the prefix.
  assert.equal(notNow("busy: conductor is still responding"), true, "conductor::BUSY");
  assert.equal(notNow("busy: the agent is still responding"), true, "the artifact agent's");
  // Whatever the words become, or the language they become it in.
  assert.equal(notNow("busy: 지휘자가 아직 응답 중입니다"), true);
  assert.equal(notNow("BUSY: still going"), true, "case is not the contract");
  assert.equal(notNow("  busy: with leading space"), true, "nor is a stray space");
  // Tauri hands a command's Err(String) through bare today, so this is the
  // message itself; an Error around it later must not turn a wait into a
  // failure, which is the exact fault the prefix exists to prevent.
  assert.equal(notNow(new Error("busy: wrapped in an Error")), true);
});

test('"cannot" is not mistaken for "not now"', () => {
  // The cost of a false positive is a message that retries for ever; of a
  // false negative, one handed back to the human as a failure.
  assert.equal(notNow("no conductor session"), false);
  assert.equal(notNow("the conductor is still responding"), false, "the old sentence, without the prefix");
  assert.equal(notNow("failed: the agent is busy: something"), false, "busy: has to be the start");
  assert.equal(notNow(""), false);
  assert.equal(notNow(null), false);
});
