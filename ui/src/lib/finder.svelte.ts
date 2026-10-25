/**
 * Finding text in a conversation on screen (Timeline, and so the agent
 * panels; WorkerView): the find bar's state, when the search runs, which
 * match is shown, and painting the matches on the drawn turns. A component
 * gives its scroller, its content, its pager and its turns as regions
 * (chatFind.ts), and draws FindBar while `open`.
 *
 * The whole conversation is searched, not just the page drawn. Matches are
 * counted over the data and painted with the CSS Custom Highlight API, so
 * nothing in the drawn text is rewritten; a match on a turn not drawn is
 * reached the way a notice reaches its decision, by the pager's bounded
 * reveal. Nothing typed here goes to any agent.
 */

import { tick, untrack } from "svelte";
import { isFindKey } from "./boardFind";
import { NO_OPTIONS, collect, jumpTo, keepIndex, piecesOf, spots, startIndex, stepIndex, stepOf, type FindOptions, type Match, type TurnRegions } from "./chatFind";
import type { StepSegment } from "./steps";

/** How long typing rests before the search runs. */
export const FIND_DEBOUNCE = 120;

/** The find bar sits over the top of the view: a match under it is not in view. */
const UNDER_BAR = 56;

const mac = typeof navigator !== "undefined" && /Mac|iPhone|iPad/.test(navigator.platform || navigator.userAgent);

export type FinderSource = {
  /** The component's own root: Ctrl+F counts inside its parent (the column with its header and composer). */
  root: () => HTMLElement | undefined;
  scroller: () => HTMLElement | undefined;
  content: () => HTMLElement | undefined;
  /** The conversation on screen; another one is a new search. */
  key: () => string;
  /** How many turns it has, oldest first. */
  total: () => number;
  /** Every turn's regions, oldest first, as the options say. Read reactively. */
  regions: (opts: FindOptions) => TurnRegions[];
  pager: { first: number; end: number; reveal: (at: number) => void };
  /** Whether the view follows the foot; and stop following it. */
  stuck: () => boolean;
  unstick: () => void;
  /** Before a match in turn `turn` is drawn: replay what it needs (its steps). */
  prepare?: (m: Match) => Promise<void>;
  /** Ctrl+F with nothing focused opens this bar (one page at a time says yes). */
  idle?: () => boolean;
  /** "Open the bar" requests, a counter (findAsk.svelte.ts). */
  asked?: () => number;
};

export function createFinder(src: FinderSource) {
  let open = $state(false);
  /** As typed. */
  let text = $state("");
  /** As searched: what was typed once typing rested. */
  let query = $state("");
  let opts = $state<FindOptions>({ ...NO_OPTIONS });
  let matches = $state.raw<Match[]>([]);
  let at = $state(-1);
  let bar: { focus: () => void } | undefined;
  const current = $derived<Match | undefined>(open && at >= 0 ? matches[at] : undefined);
  /** The current match on the page, once painted. */
  let currentRange: Range | null = null;

  $effect(() => {
    const typed = text;
    if (!typed.trim()) {
      query = "";
      return;
    }
    const timer = setTimeout(() => (query = typed), FIND_DEBOUNCE);
    return () => clearTimeout(timer);
  });

  async function show_() {
    open = true;
    await tick();
    bar?.focus();
  }

  function close() {
    open = false;
    paint();
    src.scroller()?.focus({ preventScroll: true });
  }

  if (src.asked) {
    let seen = src.asked();
    $effect(() => {
      const n = src.asked!();
      if (n === seen) return;
      seen = n;
      untrack(() => void show_());
    });
  }

  // Ctrl+F / Cmd+F inside the conversation's column, or with nothing focused
  // where `idle` allows it. The design board keeps its own find for the board.
  $effect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.defaultPrevented || !isFindKey(e, mac)) return;
      const column = src.root()?.parentElement;
      const target = e.target instanceof Node ? e.target : null;
      const here = !!column && !!target && column.contains(target);
      const nothing = !document.activeElement || document.activeElement === document.body;
      if (!here && !(nothing && (src.idle?.() ?? false))) return;
      e.preventDefault();
      void show_();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  });

  const turns = $derived.by((): TurnRegions[] => (open && query ? src.regions(opts) : []));

  /** What the last search was for: a different one starts again from the view. */
  let searched = "";
  /** Bumped by every show, so one overtaken by the next stops where it is. */
  let showing = 0;

  $effect(() => {
    const all = turns;
    const q = query;
    const how = { matchCase: opts.matchCase, wholeWord: opts.wholeWord };
    const what = [src.key(), q, how.matchCase, how.wholeWord, opts.steps].join("|");
    untrack(() => {
      performance.mark("divixi:find:start");
      const next = collect(all, q, how);
      performance.measure("divixi:find", "divixi:find:start");
      const fresh = what !== searched;
      searched = what;
      const before = matches[at];
      matches = next;
      if (fresh || !before) {
        at = startIndex(next, before?.turn ?? viewTurn());
        void reveal();
      } else {
        at = keepIndex(before, next, (m) => m.turn);
        schedulePaint();
      }
    });
  });

  /** The newest turn in view, or past them all while following the foot. */
  function viewTurn(): number {
    const box = src.scroller();
    const content = src.content();
    if (src.stuck() || !box || !content) return src.total();
    const bottom = box.getBoundingClientRect().bottom;
    let last = src.pager.first;
    for (const el of content.querySelectorAll<HTMLElement>("[data-turn]")) {
      if (el.getBoundingClientRect().top < bottom) last = Number(el.dataset.turn);
    }
    return last;
  }

  function step(back: boolean) {
    if (!matches.length) return;
    at = stepIndex(at, matches.length, back);
    void reveal();
  }

  /** Draw the current match's turn if it is not, open what folds it away, paint, and bring it into view. */
  async function reveal() {
    const ticket = ++showing;
    const m = matches[at];
    if (!m) {
      schedulePaint();
      return;
    }
    if (jumpTo(m.turn, src.pager.first, src.pager.end, src.total()) === "reveal") {
      src.pager.reveal(m.turn);
      await tick();
    }
    await src.prepare?.(m);
    await tick();
    if (ticket !== showing) return;
    paint();
    into();
  }

  /** Paint every match on the drawn turns, and the current one more strongly. */
  function paint() {
    const registry = typeof CSS !== "undefined" ? CSS.highlights : undefined;
    currentRange = null;
    if (!registry) return;
    const all: Range[] = [];
    const content = src.content();
    if (open && query && matches.length && content) {
      const counted = new Set(matches.map((m) => `${m.turn}|${m.key}`));
      const c = current;
      const how = { matchCase: opts.matchCase, wholeWord: opts.wholeWord };
      for (const el of content.querySelectorAll<HTMLElement>("[data-find]")) {
        const turnEl = el.closest<HTMLElement>("[data-turn]");
        if (!turnEl) continue;
        const turn = Number(turnEl.dataset.turn);
        const key = el.dataset.find ?? "";
        if (!counted.has(`${turn}|${key}`)) continue;
        const pieces = piecesOf(el);
        const found = spots(pieces, query, how);
        const mine = c && c.turn === turn && c.key === key ? Math.min(c.k, found.length - 1) : -1;
        found.forEach((s, k) => {
          const r = document.createRange();
          r.setStart(pieces[s.start].node!, s.startAt);
          r.setEnd(pieces[s.end].node!, s.endAt);
          if (k === mine) currentRange = r;
          else all.push(r);
        });
      }
    }
    if (all.length) registry.set("divixi-find", new Highlight(...all));
    else registry.delete("divixi-find");
    if (currentRange) registry.set("divixi-find-current", new Highlight(currentRange));
    else registry.delete("divixi-find-current");
  }

  let paintQueued = false;
  function schedulePaint() {
    if (paintQueued) return;
    paintQueued = true;
    requestAnimationFrame(() => {
      paintQueued = false;
      paint();
    });
  }

  // Whatever changes the drawn turns (a page landing, a streamed word, a
  // card opening) is painted again, a frame later.
  $effect(() => {
    const inner = src.content();
    if (!inner || !open) return;
    const observer = new MutationObserver(schedulePaint);
    observer.observe(inner, { childList: true, subtree: true, characterData: true });
    return () => observer.disconnect();
  });

  // Painted again when the matches or the current one change; cleared on close.
  $effect(() => {
    void current;
    void matches;
    void open;
    untrack(schedulePaint);
  });

  // Highlights belong to the document, not to the component: none outlive it.
  $effect(() => () => {
    CSS.highlights?.delete("divixi-find");
    CSS.highlights?.delete("divixi-find-current");
  });

  /** Bring the current match into the middle of the view, unless it is already well in it. */
  function into() {
    const box = src.scroller();
    const content = src.content();
    const m = current;
    if (!box || !content || !m) return;
    const rect = currentRange?.getBoundingClientRect() ?? content.querySelector(`[data-turn="${m.turn}"]`)?.getBoundingClientRect();
    if (!rect) return;
    const view = box.getBoundingClientRect();
    if (rect.top >= view.top + UNDER_BAR && rect.bottom <= view.bottom - 24) return;
    // Moving the view is not the conversation moving on: stop following the foot first.
    src.unstick();
    box.scrollTop += rect.top - view.top - (box.clientHeight - rect.height) / 2;
  }

  return {
    get open() {
      return open;
    },
    get text() {
      return text;
    },
    set text(v: string) {
      text = v;
    },
    get query() {
      return query;
    },
    get opts() {
      return opts;
    },
    set opts(v: FindOptions) {
      opts = v;
    },
    get matches() {
      return matches;
    },
    get at() {
      return at;
    },
    get current() {
      return current;
    },
    /** FindBar, for the focus. */
    set bar(b: { focus: () => void } | undefined) {
      bar = b;
    },
    get bar() {
      return bar;
    },
    show: show_,
    close,
    step,
    /** The step the current match names in turn `turn`'s steps block `block`, for the block to open to it. */
    foundStep(turn: number, block: number): number | undefined {
      const m = current;
      if (!m || m.turn !== turn) return undefined;
      const s = stepOf(m.key);
      return s && s.at === block ? s.i : undefined;
    },
    /** Whether the current match is in turn `turn` and `inside` says its key is there. */
    foundIn(turn: number, inside: (key: string) => boolean): boolean {
      const m = current;
      return !!m && m.turn === turn && inside(m.key);
    },
  };
}

export type Finder = ReturnType<typeof createFinder>;

/**
 * The steps of finished turns no one has replayed, from the core in one call
 * (`run_segments`): what a search over steps reads for turns not drawn.
 */
export function createStepSource(fetch: (ids: string[]) => Promise<Record<string, StepSegment[]>>) {
  let got = $state.raw<Record<string, StepSegment[]>>({});
  const asked = new Set<string>();
  return {
    of(id: string): StepSegment[] | undefined {
      return got[id];
    },
    /** Ask for the runs not asked for yet. */
    want(ids: string[]) {
      const fresh = ids.filter((id) => !asked.has(id));
      if (!fresh.length) return;
      for (const id of fresh) asked.add(id);
      void fetch(fresh).then((more) => {
        if (Object.keys(more).length) got = { ...got, ...more };
      });
    },
  };
}
