/**
 * The paging of a conversation on screen (Timeline, and so the agent panels;
 * WorkerView): which page is drawn (history.ts), when the next one is added,
 * and how the view keeps its place while it lands. A component gives its
 * scroller, its content and its state, and draws the turns `first`..`end`.
 *
 * Who scrolls: the component's stick state decides whether the view follows
 * the foot. Here the browser's own scroll anchoring is kept out of the way
 * while following, since it moved a followed foot and that read as the human
 * scrolling away. Reading back, anchoring keeps the place for whatever grows
 * above (a folded card opening, say). A page placed by hand turns it off for
 * that one placement and is then measured, so it is not taken for growth
 * below.
 */

import { tick } from "svelte";
import { NEAR_TOP, NO_WINDOW, endAt, hasEarlier, hasNewer, reach, startAt, widenDown, widenUp, type PageWindow } from "./history";

export type PagerSource = {
  scroller: () => HTMLElement | undefined;
  content: () => HTMLElement | undefined;
  /** The conversation on screen; a window belongs to one. */
  key: () => string;
  /** How many turns the conversation has, oldest first. */
  total: () => number;
  /** Whether the view is following the foot. */
  stuck: () => boolean;
  /** Stop following the foot: a window that stops short of it must not be chased. */
  unstick?: () => void;
};

export function createPager(src: PagerSource) {
  let win = $state<PageWindow>(NO_WINDOW);
  let landing = false;
  /** The content height as last read; growth is measured against it. */
  let tall = 0;
  /** Where the view was when the pager last looked; a scroll's direction is read from it. */
  let lastTop = 0;

  const first = $derived(startAt(win, src.key(), src.total()));
  const end = $derived(endAt(win, src.key(), src.total()));
  /** The window stops short of the foot: a notice drew it, and the foot is one jump away. */
  const bounded = $derived(hasNewer(win, src.key(), src.total()));

  // Set whenever following changes, but not while a page is landing: that
  // placement turns anchoring off itself and puts it back when it is done.
  $effect(() => {
    const box = src.scroller();
    const stuck = src.stuck();
    if (box && !landing) box.style.overflowAnchor = stuck ? "none" : "";
  });

  /** Make a change to the window, keeping the view in place. */
  async function land(change: () => void, above: boolean) {
    const box = src.scroller();
    const content = src.content();
    if (!box || !content || landing) return;
    landing = true;
    box.style.overflowAnchor = "none";
    const before = box.scrollHeight;
    change();
    await tick();
    // Turns landed above what is being read: move the view down by exactly that.
    if (above) box.scrollTop += box.scrollHeight - before;
    box.style.overflowAnchor = src.stuck() ? "none" : "";
    tall = content.offsetHeight;
    lastTop = box.scrollTop;
    landing = false;
  }

  /** One more page of older turns. */
  function earlier() {
    if (!hasEarlier(win, src.key(), src.total())) return;
    void land(() => (win = widenUp(win, src.key(), src.total())), true);
  }

  /** One more page of newer turns, while the window stops short of the foot. */
  function newer() {
    if (!bounded) return;
    void land(() => (win = widenDown(win, src.key(), src.total())), false);
  }

  /**
   * Called on every scroll and wheel: a view the human has read back from
   * pages on by itself, toward whichever edge it is near. Reading up pages
   * older turns in; reading down pages newer ones in, so a view that is moving
   * up (a notice bringing its card into view, say) does not page on the way.
   * One still following the foot does not page.
   */
  function check() {
    const box = src.scroller();
    if (!box) return;
    const moved = box.scrollTop - lastTop;
    lastTop = box.scrollTop;
    if (src.stuck()) return;
    if (box.scrollTop < NEAR_TOP && moved <= 0) earlier();
    else if (box.scrollHeight - box.scrollTop - box.clientHeight < NEAR_TOP && moved >= 0) newer();
  }

  /** A notice asks for the turn at index `at`: the window reaches it, and the view stops following the foot. */
  function reveal(at: number) {
    win = reach(win, src.key(), src.total(), at);
    src.unstick?.();
  }

  /** Back to the newest page, when the window is off it. Resolves once it is drawn. */
  async function toFoot() {
    if (!bounded) return;
    win = NO_WINDOW;
    await tick();
  }

  /** The content height as the observer last saw it; true if it grew since. */
  function grew(): boolean {
    const content = src.content();
    if (!content) return false;
    const h = content.offsetHeight;
    const g = h > tall;
    tall = h;
    return g;
  }

  /** Start watching the content from its current height. */
  function settle() {
    const content = src.content();
    if (content) tall = content.offsetHeight;
  }

  return {
    /** Index of the first turn drawn; the turns above it are not drawn yet. */
    get first() {
      return first;
    },
    /** One past the last turn drawn. */
    get end() {
      return end;
    },
    get bounded() {
      return bounded;
    },
    earlier,
    check,
    reveal,
    toFoot,
    grew,
    settle,
  };
}
