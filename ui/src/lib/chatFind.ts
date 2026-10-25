/**
 * Finding text in a conversation (Ctrl+F / Cmd+F, or the search button on
 * the track's header): plain text, not a question to any agent.
 *
 * The whole conversation is searched, not only the page of turns drawn
 * (pager.svelte.ts). Each turn is a list of regions, the pieces of text it
 * shows (the human's message, a paragraph of the answer, a report's summary,
 * a decision's question, a step), each with a key that is unique in its turn
 * and that the drawn region carries as `data-find`. The matches are counted
 * over that data, so "3/17" does not change as turns are drawn or folded;
 * the drawn regions are only where the matches are painted.
 *
 * Text is compared with every run of white space made one space, and case
 * aside unless asked. Pure, so `node --test` reaches it; the drawing is
 * Timeline.svelte and FindBar.svelte.
 */

import { Lexer, type Token, type Tokens } from "marked";
import { detail, shape, type Step, type StepSegment } from "./steps.ts";

export type FindOptions = {
  /** Upper and lower case are different letters. */
  matchCase: boolean;
  /** A match must not run on into a letter or digit on either side. */
  wholeWord: boolean;
  /** Include the steps a turn took (tool calls and thoughts), folded or not. */
  steps: boolean;
};

export const NO_OPTIONS: FindOptions = { matchCase: false, wholeWord: false, steps: false };

/** A piece of text a turn shows. `key` is unique within its turn. */
export type Region = { key: string; text: string };

/** A turn's regions in the order it shows them; `turn` is its index, oldest first. */
export type TurnRegions = { turn: number; regions: Region[] };

/** One match: the `k`-th (from 0) in region `key` of turn `turn`. */
export type Match = { turn: number; key: string; k: number };

/** The query as compared: composed (Korean typed in jamo matches), white space made one. Empty finds nothing. */
export function normalizeQuery(q: string, matchCase = false): string {
  const s = q.normalize("NFC").replace(/\s+/g, " ");
  if (!s.trim()) return "";
  return matchCase ? s : lower(s);
}

/** Lower case, letter by letter where one would change length (so offsets still line up). */
function lower(s: string): string {
  const l = s.toLowerCase();
  if (l.length === s.length) return l;
  let out = "";
  for (const ch of s) {
    const c = ch.toLowerCase();
    out += c.length === ch.length ? c : ch;
  }
  return out;
}

const WORD = /[\p{L}\p{N}_]/u;

/** The character starting at code unit `i`, or "" past either end. */
function charAt(s: string, i: number): string {
  if (i < 0 || i >= s.length) return "";
  return String.fromCodePoint(s.codePointAt(i)!);
}

/** The character ending just before code unit `i`, or "" at the start. */
function charBefore(s: string, i: number): string {
  if (i <= 0) return "";
  const low = s.charCodeAt(i - 1);
  return low >= 0xdc00 && low <= 0xdfff && i >= 2 ? s.slice(i - 2, i) : s[i - 1];
}

/** Whether a match at [i, i + q.length) of `s` stands as a whole word. */
function whole(s: string, i: number, q: string): boolean {
  // Only an edge of the query that is itself a letter needs a boundary: "-x" may follow a letter.
  if (WORD.test(charAt(q, 0)) && WORD.test(charBefore(s, i))) return false;
  if (WORD.test(charBefore(q, q.length)) && WORD.test(charAt(s, i + q.length))) return false;
  return true;
}

/** Where `q` (already normalized) starts in `s` (already prepared), left to right, never overlapping. */
export function positions(s: string, q: string, wholeWord: boolean): number[] {
  const out: number[] = [];
  if (!q) return out;
  let i = s.indexOf(q);
  while (i >= 0) {
    if (!wholeWord || whole(s, i, q)) {
      out.push(i);
      i = s.indexOf(q, i + q.length);
    } else i = s.indexOf(q, i + 1);
  }
  return out;
}

/** A region's text as compared: white space made one, and lower case unless `matchCase`. */
export function prepare(text: string, matchCase: boolean): string {
  const s = text.replace(/\s+/g, " ");
  return matchCase ? s : lower(s);
}

/**
 * A text prepared for comparison, with where each of its code units came
 * from in the original (one more entry for the end). What paints a match on
 * the page: the offsets lead back to the text nodes.
 */
export function prepareMapped(text: string, matchCase: boolean): { s: string; map: number[] } {
  let s = "";
  const map: number[] = [];
  let space = false;
  let i = 0;
  for (const ch of text) {
    if (/\s/.test(ch)) {
      if (!space) {
        s += " ";
        map.push(i);
      }
      space = true;
    } else {
      space = false;
      let c = matchCase ? ch : ch.toLowerCase();
      if (c.length !== ch.length) c = ch;
      s += c;
      for (let j = 0; j < c.length; j++) map.push(i + j);
    }
    i += ch.length;
  }
  map.push(i);
  return { s, map };
}

/** The [start, end) ranges of every match of `query` in `text`, in `text`'s own offsets. */
export function rangesIn(text: string, query: string, opts: Pick<FindOptions, "matchCase" | "wholeWord">): [number, number][] {
  const q = normalizeQuery(query, opts.matchCase);
  if (!q) return [];
  const { s, map } = prepareMapped(text, opts.matchCase);
  return positions(s, q, opts.wholeWord).map((i) => [map[i], map[i + q.length - 1] + 1]);
}

/**
 * Prepared texts, kept between keystrokes: a long history is hundreds of
 * regions and most of them never change. Keyed by the text itself, so a turn
 * still streaming simply makes new entries; cleared when it grows too large.
 */
const prepared = [new Map<string, string>(), new Map<string, string>()];
const KEEP = 20_000;

function cached(text: string, matchCase: boolean): string {
  const m = prepared[matchCase ? 1 : 0];
  let s = m.get(text);
  if (s === undefined) {
    if (m.size >= KEEP) m.clear();
    s = prepare(text, matchCase);
    m.set(text, s);
  }
  return s;
}

/** Every match of `query` in the conversation, in reading order: turn by turn, region by region. */
export function collect(turns: TurnRegions[], query: string, opts: Pick<FindOptions, "matchCase" | "wholeWord">): Match[] {
  const q = normalizeQuery(query, opts.matchCase);
  const out: Match[] = [];
  if (!q) return out;
  for (const t of turns) {
    for (const r of t.regions) {
      if (!r.text) continue;
      const s = cached(r.text, opts.matchCase);
      if (s.length < q.length) continue;
      const n = positions(s, q, opts.wholeWord).length;
      for (let k = 0; k < n; k++) out.push({ turn: t.turn, key: r.key, k });
    }
  }
  return out;
}

/**
 * Where a new search starts: the last match at or above `turn` (the newest
 * turn in view), so the first one shown is the nearest one the human has
 * already passed, reading up. Below it all: the first match. None: -1.
 */
export function startIndex(matches: Match[], turn: number): number {
  if (!matches.length) return -1;
  for (let i = matches.length - 1; i >= 0; i--) if (matches[i].turn <= turn) return i;
  return 0;
}

/**
 * Where the same match is after the matches changed under it (a turn
 * arrived, a toggle flipped): the same turn, region and number if it is
 * still there, else the first one after where it was, else the last.
 */
export function keepIndex(before: Match | undefined, after: Match[], order: (m: Match) => number): number {
  if (!after.length) return -1;
  if (!before) return -1;
  const same = after.findIndex((m) => m.turn === before.turn && m.key === before.key && m.k === before.k);
  if (same >= 0) return same;
  const at = order(before);
  const next = after.findIndex((m) => order(m) >= at);
  return next >= 0 ? next : after.length - 1;
}

/** The next match from `at` (-1: none yet), going forward or back, round. */
export function stepIndex(at: number, count: number, back: boolean): number {
  if (count <= 0) return -1;
  if (at < 0) return back ? count - 1 : 0;
  return (at + (back ? -1 : 1) + count) % count;
}

/**
 * What it takes to show a match in turn `turn`, with turns `first`..`end`
 * drawn of `total`: nothing ("drawn"), or a window around it ("reveal", the
 * pager's bounded reveal that notices use). Turn `total` is the decisions
 * asked by no turn, drawn under them all.
 */
export function jumpTo(turn: number, first: number, end: number, total: number): "drawn" | "reveal" {
  if (turn >= total) return "drawn";
  return turn >= first && turn < end ? "drawn" : "reveal";
}

/** What "3/17" says; "" while there is nothing to count. */
export function countLabel(at: number, count: number): { i: number; n: number } | null {
  if (count <= 0) return null;
  return { i: Math.max(0, at) + 1, n: count };
}

// ----- what a turn shows, as text -----

/** Hook chatter (devterm memory and the like) arrives as message text; it is not shown. */
export function cleanText(text: string): string {
  // "Notice: <Hook> says: …" up to the next notice or line end; they
  // often arrive glued together on one line.
  return text
    .replace(/Notice: [\w:.-]+ says: .*?(?=Notice: [\w:.-]+ says: |\n|$)/g, "")
    .replace(/[ \t]+\n/g, "\n")
    .replace(/\n{3,}/g, "\n\n");
}

const ENTITIES: Record<string, string> = { amp: "&", lt: "<", gt: ">", quot: '"', apos: "'", nbsp: " " };

/** `&amp;` and `&#39;` as the page shows them. */
function entities(s: string): string {
  return s.replace(/&(#x[0-9a-f]+|#\d+|[a-z]+);/gi, (all, e: string) => {
    if (e[0] === "#") {
      const code = e[1] === "x" || e[1] === "X" ? parseInt(e.slice(2), 16) : parseInt(e.slice(1), 10);
      return Number.isFinite(code) && code > 0 && code < 0x110000 ? String.fromCodePoint(code) : all;
    }
    return ENTITIES[e.toLowerCase()] ?? all;
  });
}

function inline(tokens: Token[] | undefined, out: string[]) {
  for (const t of tokens ?? []) {
    switch (t.type) {
      case "br":
        out.push(" ");
        break;
      case "codespan":
      case "escape":
        out.push(t.text);
        break;
      case "image": {
        // Markdown.svelte turns a picture from the web into a link to it; a
        // local one shows no text.
        const img = t as Tokens.Image;
        if (/^https?:/i.test(img.href)) out.push(img.text || img.href);
        break;
      }
      case "html":
        out.push(entities(t.text.replace(/<[^>]*>/g, "")));
        break;
      case "checkbox":
        break;
      default:
        if ("tokens" in t && t.tokens) inline(t.tokens, out);
        else if ("text" in t && typeof t.text === "string") out.push(entities(t.text));
    }
  }
}

function blocks(tokens: Token[], out: string[]) {
  for (const t of tokens) {
    switch (t.type) {
      case "space":
      case "hr":
      case "def":
        break;
      case "code":
        out.push(t.text);
        break;
      case "html":
        out.push(entities(t.text.replace(/<[^>]*>/g, "")));
        break;
      case "list":
        for (const item of (t as Tokens.List).items) {
          blocks(item.tokens, out);
          out.push(" ");
        }
        break;
      case "table": {
        const table = t as Tokens.Table;
        for (const row of [table.header, ...table.rows]) {
          for (const cell of row) {
            inline(cell.tokens, out);
            out.push(" ");
          }
        }
        break;
      }
      case "blockquote":
        blocks((t as Tokens.Blockquote).tokens, out);
        break;
      case "paragraph":
      case "heading":
        inline(t.tokens, out);
        break;
      case "text":
        // A list item's text: inline tokens of its own.
        if (t.tokens) inline(t.tokens, out);
        else out.push(entities(t.text));
        break;
      default:
        if ("tokens" in t && t.tokens) inline(t.tokens, out);
        else if ("text" in t && typeof t.text === "string") out.push(entities(t.text));
    }
    out.push(" ");
  }
}

const plain = new Map<string, string>();

/**
 * Markdown as the page reads it once drawn (Markdown.svelte): the words,
 * without the marks around them, the address of a link, or the fence of a
 * code block. Kept by source, since an answer is drawn from the same text
 * every time.
 */
export function markdownText(source: string): string {
  let s = plain.get(source);
  if (s !== undefined) return s;
  const out: string[] = [];
  try {
    blocks(Lexer.lex(source, { gfm: true, breaks: true }), out);
    s = out.join("");
  } catch {
    s = source;
  }
  if (plain.size >= KEEP) plain.clear();
  plain.set(source, s);
  return s;
}

// ----- the page: regions as drawn -----

/** Elements that start a line of their own: a gap between their text and the next. */
const BLOCK = new Set(["P", "DIV", "LI", "UL", "OL", "PRE", "H1", "H2", "H3", "H4", "H5", "H6", "TABLE", "TR", "TD", "TH", "BLOCKQUOTE", "HR", "BR", "SECTION"]);

/** A drawn region's text, piece by piece: a text node, or the gap a block or a line break makes (`null`). */
export type Piece<N> = { node: N | null; text: string };

/** Where a match lands on the pieces: the piece and offset it starts at and ends at. */
export type Spot = { start: number; startAt: number; end: number; endAt: number };

/**
 * Matches in a region drawn as `pieces`, as places on them. A match that
 * starts or ends on a gap is moved onto the text beside it.
 */
export function spots<N>(pieces: Piece<N>[], query: string, opts: Pick<FindOptions, "matchCase" | "wholeWord">): Spot[] {
  const text = pieces.map((p) => p.text).join("");
  const ranges = rangesIn(text, query, opts);
  if (!ranges.length) return [];
  // Start offset of each piece in `text`.
  const starts: number[] = [];
  let at = 0;
  for (const p of pieces) {
    starts.push(at);
    at += p.text.length;
  }
  const pieceAt = (offset: number, forEnd: boolean): number => {
    // The last piece starting at or before the offset (for an end: before it).
    let lo = 0;
    let hi = pieces.length - 1;
    let found = 0;
    while (lo <= hi) {
      const mid = (lo + hi) >> 1;
      if (forEnd ? starts[mid] < offset : starts[mid] <= offset) {
        found = mid;
        lo = mid + 1;
      } else hi = mid - 1;
    }
    return found;
  };
  const out: Spot[] = [];
  for (const [a, b] of ranges) {
    let s = pieceAt(a, false);
    let sOff = a - starts[s];
    while (s < pieces.length && (pieces[s].node === null || sOff >= pieces[s].text.length)) {
      s++;
      sOff = 0;
    }
    let e = pieceAt(b, true);
    let eOff = b - starts[e];
    while (e >= 0 && pieces[e].node === null) {
      e--;
      eOff = e >= 0 ? pieces[e].text.length : 0;
    }
    if (s >= pieces.length || e < 0 || e < s || (e === s && eOff <= sOff)) continue;
    out.push({ start: s, startAt: sOff, end: e, endAt: eOff });
  }
  return out;
}

/** A drawn region's pieces: its text nodes in order, with a gap at each block and line break. */
export function piecesOf(root: Node): Piece<Text>[] {
  const out: Piece<Text>[] = [];
  const walk = (n: Node) => {
    for (let c = n.firstChild; c; c = c.nextSibling) {
      if (c.nodeType === 3) {
        const text = (c as Text).data;
        if (text) out.push({ node: c as Text, text });
      } else if (c.nodeType === 1) {
        const tag = (c as Element).tagName;
        const block = BLOCK.has(tag);
        if (block) out.push({ node: null, text: " " });
        walk(c);
        if (block) out.push({ node: null, text: " " });
      }
    }
  };
  walk(root);
  return out;
}

// ----- a turn's regions, in the order the timeline draws them -----

/** The part of `store.WorkerReport` a search reads. */
export type FindReport = {
  summary: string;
  questions: string[];
  changes: { path: string; what: string }[];
  checks: { what: string; detail: string }[];
  risks: string[];
  next: string[];
};

/** The part of `store.Decision` a search reads. */
export type FindDecision = {
  id: number;
  status: string;
  question: string;
  context: string;
  options: { label: string; detail?: string }[];
  choice: number | null;
  answer: string | null;
  note: string;
};

/** A conversation turn as a search reads it; see `turnRegions`. */
export type FindTurn = {
  turn: number;
  /**
   * What opens the turn: the human's message, a worker's report, or a system
   * line; in a worker's view, what the conductor sent it (markdown).
   */
  head: { kind: "me" | "line" | "markdown"; text: string } | { kind: "report"; report: FindReport | undefined };
  /**
   * The answer's pieces in order, when known (a live turn, a replayed one,
   * or one the core sent for a search over steps); otherwise `message`,
   * the folded text a restored turn shows.
   */
  segments: StepSegment[] | null;
  message: string;
  /** A worker's own report, drawn under its answer in the worker's view. */
  report?: FindReport;
  /** The failure shown under the answer, if any. */
  error: string;
  decisions: FindDecision[];
};

/** The region keys the drawn turn carries as `data-find`. */
export const findKey = {
  said: "p",
  line: "x",
  message: "m",
  error: "e",
  text: (block: number) => `t${block}`,
  step: (at: number, i: number) => `s${at}.${i}`,
  report: { summary: "r.s", question: (i: number) => `r.q${i}`, change: (i: number) => `r.c${i}`, changeWhat: (i: number) => `r.c${i}w`, check: (i: number) => `r.k${i}`, checkDetail: (i: number) => `r.k${i}d`, risk: (i: number) => `r.r${i}`, next: (i: number) => `r.n${i}` },
  decision: { question: (id: number) => `d${id}.q`, context: (id: number) => `d${id}.c`, option: (id: number, i: number) => `d${id}.o${i}`, optionDetail: (id: number, i: number) => `d${id}.o${i}d`, answer: (id: number) => `d${id}.a`, note: (id: number) => `d${id}.n` },
};

/** A key in a report's lists, which show only once the card is opened. */
export function inReportDetails(key: string): boolean {
  return /^r\.[ckrn]\d/.test(key);
}

/** The steps block and step a key names, if it names one. */
export function stepOf(key: string): { at: number; i: number } | null {
  const m = /^s(\d+)\.(\d+)$/.exec(key);
  return m ? { at: Number(m[1]), i: Number(m[2]) } : null;
}

/** A step's words as the step list shows them. */
function stepText(s: Step): string {
  return s.kind === "thought" ? s.text : detail(s.tool);
}

/** A decision card's regions, as DecisionCard.svelte draws it in each state. */
export function decisionRegions(d: FindDecision): Region[] {
  const k = findKey.decision;
  const out: Region[] = [{ key: k.question(d.id), text: d.question }];
  if (d.status === "open") {
    if (d.context) out.push({ key: k.context(d.id), text: d.context });
    d.options.forEach((o, i) => {
      out.push({ key: k.option(d.id, i), text: o.label });
      if (o.detail) out.push({ key: k.optionDetail(d.id, i), text: o.detail });
    });
  } else if (d.status === "decided") {
    const chosen = d.choice === null ? null : d.options[d.choice];
    out.push({ key: k.answer(d.id), text: chosen && d.choice !== null ? chosen.label : (d.answer ?? "") });
    if (d.note) out.push({ key: k.note(d.id), text: d.note });
    if (d.options.length > 1) d.options.forEach((o, i) => i !== d.choice && out.push({ key: k.option(d.id, i), text: o.label }));
  }
  return out;
}

/** A report card's regions: the summary and questions, then the lists it opens to. */
export function reportRegions(r: FindReport): Region[] {
  const k = findKey.report;
  const out: Region[] = [{ key: k.summary, text: r.summary }];
  r.questions.forEach((q, i) => out.push({ key: k.question(i), text: q }));
  r.changes.forEach((c, i) => {
    out.push({ key: k.change(i), text: c.path });
    if (c.what) out.push({ key: k.changeWhat(i), text: c.what });
  });
  r.checks.forEach((c, i) => {
    out.push({ key: k.check(i), text: c.what });
    if (c.detail) out.push({ key: k.checkDetail(i), text: c.detail });
  });
  r.risks.forEach((x, i) => out.push({ key: k.risk(i), text: x }));
  r.next.forEach((x, i) => out.push({ key: k.next(i), text: x }));
  return out;
}

/**
 * A turn's regions in the order Timeline.svelte (or WorkerView.svelte) draws
 * them: what opened it, the answer (paragraphs, and the steps between them
 * when `steps`), a worker's own report, the failure under it, and the
 * decisions it asked.
 */
export function turnRegions(t: FindTurn, steps: boolean): Region[] {
  const out: Region[] = [];
  if (t.head.kind === "report") {
    if (t.head.report) out.push(...reportRegions(t.head.report));
  } else if (t.head.kind === "markdown") {
    if (t.head.text.trim()) out.push({ key: findKey.said, text: markdownText(t.head.text) });
  } else if (t.head.text.trim()) out.push({ key: t.head.kind === "me" ? findKey.said : findKey.line, text: t.head.text });
  if (t.segments && t.segments.length) {
    shape(t.segments, (text) => !!cleanText(text).trim()).forEach((b, i) => {
      if (b.kind === "text") out.push({ key: findKey.text(i), text: markdownText(cleanText(b.text)) });
      else if (steps) b.steps.forEach((s, j) => out.push({ key: findKey.step(b.at, j), text: stepText(s) }));
    });
  } else if (t.message.trim()) out.push({ key: findKey.message, text: markdownText(cleanText(t.message)) });
  if (t.report) out.push(...reportRegions(t.report));
  if (t.error) out.push({ key: findKey.error, text: t.error });
  for (const d of t.decisions) out.push(...decisionRegions(d));
  return out;
}
