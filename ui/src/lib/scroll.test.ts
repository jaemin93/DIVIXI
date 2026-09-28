/** The stick-to-bottom rules, without a browser: `node --test`. */

import { strict as assert } from "node:assert";
import { test } from "node:test";
import {
  BOTTOM_SLACK,
  FOLLOWING,
  atBottom,
  bottomOf,
  followed,
  fromBottom,
  reached,
  resized,
  scrolled,
  type ScrollMetrics,
  type Stick,
} from "./scroll.ts";

/** A view 500 tall over `tall` of content, scrolled to `top`. */
function view(tall: number, top: number): ScrollMetrics {
  return { scrollHeight: tall, scrollTop: top, clientHeight: 500 };
}

/** Reading the newest of 2000px of content in a 500px view. */
const AT_FOOT = view(2000, 1500);

// ----- where the foot is -----

test("exactly at the foot", () => {
  assert.equal(fromBottom(AT_FOOT), 0);
  assert.equal(atBottom(AT_FOOT), true);
  assert.equal(bottomOf(AT_FOOT), 1500);
});

test("a few pixels short still counts as the foot", () => {
  // The point of the slack: a stray pixel from a font or a zoom level must
  // not quietly turn following off.
  assert.equal(atBottom(view(2000, 1499)), true);
  assert.equal(atBottom(view(2000, 1500 - BOTTOM_SLACK)), true);
});

test("just past the slack is reading back, and is left alone", () => {
  assert.equal(atBottom(view(2000, 1500 - BOTTOM_SLACK - 1)), false);
  assert.equal(atBottom(view(2000, 0)), false);
});

test("content shorter than the view is always at the foot", () => {
  assert.equal(atBottom(view(300, 0)), true);
  assert.equal(bottomOf(view(300, 0)), 0, "and there is nowhere to scroll to");
});

// ----- content arriving -----

test("at the foot, anything arriving is followed", () => {
  const { next, follow } = resized({ ...FOLLOWING }, true);
  assert.equal(follow, true);
  assert.equal(next.stuck, true);
  assert.equal(next.missed, false, "nothing is missed by someone watching it arrive");
});

test("at the foot, content shrinking is followed too", () => {
  // A waiting-report line disappearing when its report lands: the view
  // must stay at the foot rather than be left hanging below the content.
  const { follow } = resized({ ...FOLLOWING }, false);
  assert.equal(follow, true);
});

test("reading back, content arriving is offered, not forced", () => {
  const reading: Stick = { stuck: false, missed: false, lastTop: 400 };
  const { next, follow } = resized(reading, true);
  assert.equal(follow, false, "never drag the view off what they are reading");
  assert.equal(next.missed, true, "but say that something came in below");
  assert.equal(next.lastTop, 400, "and do not pretend the view moved");
});

test("reading back, content shrinking is not something missed", () => {
  const reading: Stick = { stuck: false, missed: false, lastTop: 400 };
  assert.equal(resized(reading, false).next.missed, false);
});

// ----- the bug this file exists for -----

test("a scroll event that lands after more content does not let go of the foot", () => {
  // The report path grows the conversation in steps: the turn appears, the
  // waiting line goes, the report card mounts, the card fills in once the
  // report has been read. We scroll to the foot on step one; the browser
  // delivers that scroll event later, by which time step three has made
  // the content 400px taller. The view has not moved — nobody touched it —
  // so reading "400px from the bottom" as the human scrolling up is wrong,
  // and used to turn following off for the rest of the conversation.
  const afterFollow = followed({ ...FOLLOWING }, 1500);
  const late = view(2400, 1500);
  assert.equal(atBottom(late), false, "the position does look far from the foot");
  const s = scrolled(afterFollow, late);
  assert.equal(s.stuck, true, "but the view never moved, so it is still following");
  assert.equal(s.missed, false);
});

test("the same, in the small steps streaming arrives in", () => {
  let s = followed({ ...FOLLOWING }, 1500);
  let tall = 2000;
  for (let i = 0; i < 50; i++) {
    tall += 30;
    // Content grows; we are stuck, so we follow and land at the new foot.
    const { follow } = resized(s, true);
    assert.equal(follow, true, `stopped following at step ${i}`);
    s = followed(s, tall - 500);
    // Every one of those scrolls is reported back, some of them late.
    s = scrolled(s, view(tall, tall - 500));
    s = scrolled(s, view(tall, tall - 500 - 30));
  }
  assert.equal(s.stuck, true);
});

test("under page zoom, a fraction of a pixel is not the human moving it", () => {
  // scrollTop is fractional when the app is zoomed; exact comparison
  // would call our own scrolling somebody else's.
  const s = followed({ ...FOLLOWING }, 1500.4);
  assert.equal(scrolled(s, view(2400, 1500.6)).stuck, true);
});

// ----- the human moving it -----

test("scrolling up lets go of the foot", () => {
  const s = scrolled(followed({ ...FOLLOWING }, 1500), view(2000, 900));
  assert.equal(s.stuck, false);
  assert.equal(s.lastTop, 900);
});

test("scrolling back down takes the foot again and clears what was missed", () => {
  const reading: Stick = { stuck: false, missed: true, lastTop: 900 };
  const s = scrolled(reading, AT_FOOT);
  assert.equal(s.stuck, true);
  assert.equal(s.missed, false);
  assert.equal(s.lastTop, 1500);
});

test("scrolling up a little, but not past the slack, keeps the foot", () => {
  const s = scrolled(followed({ ...FOLLOWING }, 1500), view(2000, 1500 - BOTTOM_SLACK + 10));
  assert.equal(s.stuck, true);
});

test("reading back, a scroll that stays put changes nothing", () => {
  const reading: Stick = { stuck: false, missed: true, lastTop: 400 };
  assert.deepEqual(scrolled(reading, view(3000, 400)), reading);
});

test("the jump-to-newest button ends where following begins", () => {
  const reading: Stick = { stuck: false, missed: true, lastTop: 400 };
  const s = followed(reading, bottomOf(view(2000, 400)));
  assert.deepEqual(s, { stuck: true, missed: false, lastTop: 1500 });
  // And the scroll event it causes does not immediately undo it, even if
  // the conversation grew again in the meantime.
  assert.equal(scrolled(s, view(2600, 1500)).stuck, true);
});

// ----- the human reaching back -----

test("a wheel pulled back lets go of the foot at once, however small the nudge", () => {
  // The bug: a nudge lands inside the slack, so the position still reads as
  // "at the foot"; the next thing to arrive follows it and the nudge is
  // undone. Fifty notches later the view has not moved at all. The wheel
  // itself is the signal, so it does not have to clear the slack first.
  const s = reached(followed({ ...FOLLOWING }, 1500), AT_FOOT, true);
  assert.equal(s.stuck, false);
  assert.equal(s.missed, false, "nothing has been missed at the moment they look up");
});

test("reaching back holds while the stream keeps arriving", () => {
  let s = reached(followed({ ...FOLLOWING }, 1500), AT_FOOT, true);
  let tall = 2000;
  // The wheel moved the view 8px up; the event for it arrives after content has.
  for (let i = 0; i < 50; i++) {
    tall += 30;
    const { next, follow } = resized(s, true);
    assert.equal(follow, false, `followed the foot anyway at step ${i}`);
    s = next;
    s = scrolled(s, view(tall, 1492));
  }
  assert.equal(s.stuck, false, "still where they left it");
  assert.equal(s.missed, true, "and told that there is more below");
});

test("scrolling down to the foot takes it back", () => {
  let s = reached(followed({ ...FOLLOWING }, 1500), AT_FOOT, true);
  s = scrolled(s, view(2000, 1400));
  assert.equal(s.stuck, false);
  s = scrolled(s, view(2000, 1500 - BOTTOM_SLACK + 4));
  assert.equal(s.stuck, true, "near enough the foot, moving toward it");
  assert.equal(s.missed, false);
});

test("above the foot, a nudge up does not take it back", () => {
  // Reading back, 20px above the foot: inside the slack, but moving away
  // from it. Re-arming here is what put the view back under the stream.
  const reading: Stick = { stuck: false, missed: true, lastTop: 1490 };
  const s = scrolled(reading, view(2000, 1470));
  assert.equal(s.stuck, false);
  assert.equal(s.missed, true);
});

test("a wheel pushed forward is not reaching back", () => {
  const foot = followed({ ...FOLLOWING }, 1500);
  assert.deepEqual(reached(foot, AT_FOOT, false), foot);
});

test("there is nothing to reach back to in a conversation shorter than the view", () => {
  const foot = followed({ ...FOLLOWING }, 0);
  assert.deepEqual(reached(foot, view(300, 0), true), foot, "and no jump button either");
});

test("reaching back twice in one gesture does not move the mark", () => {
  const s = reached(followed({ ...FOLLOWING }, 1500), AT_FOOT, true);
  assert.deepEqual(reached(s, view(2000, 1400), true), s, "the gesture is already theirs");
});

test("content shrinking under someone reading back leaves them following again", () => {
  // They folded a long report card. The content is now shorter than where
  // they were, so the browser clamps the view to the foot -- upward, and
  // with no downward move left to make. Refusing the foot here would strand
  // them: at the bottom, not following, with a jump button that goes nowhere.
  const reading: Stick = { stuck: false, missed: true, lastTop: 1500 };
  const s = scrolled(reading, view(1200, 700));
  assert.equal(fromBottom(view(1200, 700)), 0, "clamped onto the foot");
  assert.equal(s.stuck, true);
  assert.equal(s.missed, false);
});

test("a shrink that stops short of the foot is still reading back", () => {
  const reading: Stick = { stuck: false, missed: true, lastTop: 1500 };
  assert.equal(scrolled(reading, view(3000, 1400)).stuck, false);
});
