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
