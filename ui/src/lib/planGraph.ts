/**
 * A routine's plan as the window reads it: the shapes the core sends
 * (src-tauri/src/plan.rs), and what the screens make of them — the waves,
 * where each step sits on the graph, which steps a step may wait for, and
 * what the agent's suggestions did. Kept apart from the components so it is
 * tested.
 */

export type OnFail = "stop" | "continue";

/** Mirrors `plan::Step`. */
export type PlanStep = {
  id: string;
  title: string;
  instruction: string;
  /** The steps that must be done before this one. */
  after: string[];
  inputs: string[];
  outputs: string[];
  done_when: string;
  agent: string;
  on_fail: OnFail;
  /** Where a person put it; absent is laid out automatically. */
  x?: number;
  y?: number;
  by: string;
};

/** Mirrors `plan::Change`: an agent edit the human has not kept or reverted. */
export type PlanChange = {
  id: number;
  target: string;
  run: string | null;
  before: PlanStep | null;
  after: PlanStep | null;
};

/** Mirrors `plan::Doc`. */
export type PlanDoc = {
  version: number;
  routine: string;
  steps: PlanStep[];
  changes: PlanChange[];
  next: number;
  source: { instruction: string; at: number } | null;
  proposal?: { text: string; run: string | null; at: number } | null;
  /** Saved versions, oldest first. */
  saved?: PlanVersion[];
  /** The version the working plan was last opened from or saved as. */
  opened?: number | null;
  /** The version runs are given: the one last saved or saved over (absent: the last). */
  run?: number | null;
};

/** Mirrors `plan::Version`. */
export type PlanVersion = { n: number; at: number; note: string; steps: PlanStep[]; edited_at?: number | null };

/** Mirrors `plan::PlanEvent`: the plan whole after a change. */
export type PlanEvent = { id: string; base: number; doc: PlanDoc };

/** What a run's agent said of one step (the report's `steps`). */
export type StepResult = { id: string; status: "done" | "failed" | "skipped"; note?: string };

export const emptyPlan = (): PlanDoc => ({
  version: 0,
  routine: "",
  steps: [],
  changes: [],
  next: 0,
  source: null,
  proposal: null,
  saved: [],
});

const byId = (steps: PlanStep[]) => new Map(steps.map((s) => [s.id, s]));

/**
 * The waves: steps grouped by the longest way back to a step that waits for
 * nothing. Everything in a wave can go together once the waves before it
 * are done. Steps keep their plan order within a wave. A wait on a step
 * that is not there is ignored; a cycle (which the core refuses) still ends.
 */
export function waves(steps: PlanStep[]): string[][] {
  const ids = new Set(steps.map((s) => s.id));
  const depth = new Map<string, number>();
  for (let pass = 0; pass <= steps.length; pass++) {
    let moved = false;
    for (const s of steps) {
      let d = 0;
      for (const a of s.after) if (ids.has(a) && depth.has(a)) d = Math.max(d, depth.get(a)! + 1);
      d = Math.min(d, steps.length);
      if (depth.get(s.id) !== d) {
        depth.set(s.id, d);
        moved = true;
      }
    }
    if (!moved) break;
  }
  const out: string[][] = [];
  for (const s of steps) {
    const d = depth.get(s.id) ?? 0;
    (out[d] ??= []).push(s.id);
  }
  return out.filter((w) => w && w.length);
}

/** The arrows: one per wait, from the step waited for to the one waiting. */
export function arrows(steps: PlanStep[]): { from: string; to: string }[] {
  const ids = new Set(steps.map((s) => s.id));
  return steps.flatMap((s) => s.after.filter((a) => ids.has(a)).map((a) => ({ from: a, to: s.id })));
}

export const CARD_W = 220;
export const CARD_H = 104;
export const GAP_X = 80;
export const GAP_Y = 36;

/**
 * Where each step sits: a column per wave, left to right; within a column,
 * steps ordered by where the steps they wait for sit (a few sweeps of the
 * barycentre), and the column centred on the tallest.
 *
 * A step a person put somewhere stays there -- while that place still
 * reads in order: to the right of every step it waits for and to the left
 * of every step that waits for it. Once its steps before change so that
 * the place would put it out of order, the place is let go and the layout
 * places it (the "lay out did nothing" report: a card pinned by an earlier
 * drag kept its spot through a change of order).
 */
export function layout(steps: PlanStep[]): Map<string, { x: number; y: number }> {
  const ws = waves(steps);
  const index = byId(steps);
  const row = new Map<string, number>();
  ws.forEach((w) => w.forEach((id, k) => row.set(id, k)));
  for (let sweep = 0; sweep < 4; sweep++) {
    ws.forEach((w, i) => {
      if (!i) return;
      const centre = (id: string) => {
        const before = index.get(id)!.after.filter((a) => row.has(a));
        return before.length ? before.reduce((sum, a) => sum + row.get(a)!, 0) / before.length : row.get(id)!;
      };
      // Sorted by a key read before any row moves, so the sort is stable.
      const keyed = w.map((id, k) => ({ id, key: centre(id), k }));
      keyed.sort((a, b) => a.key - b.key || a.k - b.k);
      w.splice(0, w.length, ...keyed.map((x) => x.id));
      w.forEach((id, k) => row.set(id, k));
    });
  }
  const tallest = Math.max(1, ...ws.map((w) => w.length));
  const auto = new Map<string, { x: number; y: number }>();
  ws.forEach((w, col) => {
    const offset = ((tallest - w.length) * (CARD_H + GAP_Y)) / 2;
    w.forEach((id, k) => auto.set(id, { x: col * (CARD_W + GAP_X), y: offset + k * (CARD_H + GAP_Y) }));
  });
  // Where each step ends up: its pin when the pin still reads in order.
  // Wave by wave, each pin is read against where the steps it waits for
  // ended up (their pins already settled), and against the steps waiting
  // for it that are laid out; a pinned step waiting for it is judged in its
  // own turn, against where this one ended up. So one pin out of order lets
  // go of itself, not of the pins around it. A pin let go can still leave
  // an earlier pin to the right of a step now laid out: then that pin goes
  // too, and the layout is read again.
  const released = new Set<string>();
  const pinnedAt = (id: string) => {
    const s = index.get(id)!;
    return s.x !== undefined && s.y !== undefined && !released.has(id);
  };
  const waiting = (id: string) => steps.filter((o) => o.after.includes(id)).map((o) => o.id);
  for (let pass = 0; ; pass++) {
    const out = new Map(auto);
    for (const w of ws) {
      for (const id of w) {
        if (!pinnedAt(id)) continue;
        const s = index.get(id)!;
        const rightOfBefore = s.after.filter((a) => out.has(a)).every((a) => s.x! > out.get(a)!.x);
        const leftOfLaidOut = waiting(id)
          .filter((o) => !pinnedAt(o))
          .every((o) => s.x! < auto.get(o)!.x);
        if (rightOfBefore && leftOfLaidOut) out.set(id, { x: s.x!, y: s.y! });
        else released.add(id);
      }
    }
    const late = [...out.keys()].filter((id) => pinnedAt(id) && waiting(id).some((o) => out.get(o)!.x <= out.get(id)!.x));
    if (!late.length || pass > steps.length) return out;
    late.forEach((id) => released.add(id));
  }
}

/**
 * Whether making `id` wait for `other` would close a cycle: `other` already
 * waits, directly or not, for `id`.
 */
export function closesCycle(steps: PlanStep[], id: string, other: string): boolean {
  if (id === other) return true;
  const index = byId(steps);
  const seen = new Set<string>();
  const stack = [other];
  while (stack.length) {
    const v = stack.pop()!;
    if (v === id) return true;
    if (seen.has(v)) continue;
    seen.add(v);
    stack.push(...(index.get(v)?.after ?? []));
  }
  return false;
}

/**
 * The "steps before this one" checklist: every other step, ticked when this
 * one waits for it, and closed (with the reason) when ticking it would make
 * a cycle. A ticked one is never closed: it can always be taken away.
 */
export function beforeChoices(steps: PlanStep[], id: string): { id: string; title: string; on: boolean; closed: boolean }[] {
  const me = steps.find((s) => s.id === id);
  if (!me) return [];
  return steps
    .filter((s) => s.id !== id)
    .map((s) => {
      const on = me.after.includes(s.id);
      return { id: s.id, title: s.title, on, closed: !on && closesCycle(steps, id, s.id) };
    });
}

/** What the agent's suggestion is to each step it touched. */
export type SuggestionKind = "added" | "changed" | "removed";

export function suggestions(doc: PlanDoc): Map<string, { change: PlanChange; kind: SuggestionKind }> {
  const out = new Map<string, { change: PlanChange; kind: SuggestionKind }>();
  for (const c of doc.changes) {
    let kind: SuggestionKind = "changed";
    if (!c.before) kind = "added";
    else if (!c.after) kind = "removed";
    out.set(c.target, { change: c, kind });
  }
  return out;
}

/** What a changed step's suggestion changed, by field, for the card to say. */
export function changedFields(c: PlanChange): string[] {
  if (!c.before || !c.after) return [];
  const fields: (keyof PlanStep)[] = ["title", "instruction", "after", "inputs", "outputs", "done_when", "on_fail", "agent"];
  return fields.filter((f) => JSON.stringify(c.before![f]) !== JSON.stringify(c.after![f]));
}

/**
 * The plan as the human sees it while suggestions wait: the steps as they
 * are, plus the ones the agent removed (still shown, struck through, so the
 * removal can be reverted), in plan order.
 */
export function shownSteps(doc: PlanDoc): { step: PlanStep; removed: boolean }[] {
  const out = doc.steps.map((step) => ({ step, removed: false }));
  for (const c of doc.changes) {
    if (c.before && !c.after) out.push({ step: c.before, removed: true });
  }
  return out;
}

/** The steps as a run would be given them: where they sit on the graph does not count. */
function content(steps: PlanStep[]): string {
  return JSON.stringify(steps.map(({ x: _x, y: _y, ...rest }) => rest));
}

/**
 * The version runs are given, as `plan::Doc::run_version` reads it: the one
 * last saved or saved over (the newest, when the plan does not say or that
 * version was let go).
 */
export function runVersion(doc: PlanDoc): PlanVersion | undefined {
  const saved = doc.saved ?? [];
  return saved.find((v) => v.n === doc.run) ?? saved.at(-1);
}

/**
 * Whether the working plan differs from the version runs are given, in what
 * a run would get (where steps sit does not count). A plan never saved
 * differs once it has steps. What "Unsaved" says, and what Run asks about.
 */
export function unsaved(doc: PlanDoc): boolean {
  const given = runVersion(doc);
  if (!given) return doc.steps.length > 0;
  return content(given.steps) !== content(doc.steps);
}

/** Whether "save as new" has something to save: the working plan is not the newest version. */
export function differsFromNewest(doc: PlanDoc): boolean {
  const last = doc.saved?.at(-1);
  if (!last) return doc.steps.length > 0;
  return content(last.steps) !== content(doc.steps);
}

/** Whether the working plan differs from saved version `n` (true when there is no such version). */
export function differsFromVersion(doc: PlanDoc, n: number): boolean {
  const v = doc.saved?.find((x) => x.n === n);
  return !v || content(v.steps) !== content(doc.steps);
}

/** What "save over" writes into: the version the working plan came from, else the last. */
export function saveTarget(doc: PlanDoc): number | undefined {
  const saved = doc.saved ?? [];
  const opened = doc.opened;
  if (typeof opened === "number" && saved.some((v) => v.n === opened)) return opened;
  return saved.at(-1)?.n;
}

/**
 * What pressing Run does with the plan on screen -- the same rule as
 * `plan::Doc::gate`, which the conductor's `run_routine` refuses by
 * (tested on the same cases on both sides):
 * - "settle": suggestions wait, so it cannot be saved: settle them first;
 * - "run": straight away (no plan, or the working plan is the run version);
 * - "askSave": it has steps but was never saved -- save and run, or not;
 * - "askUnsaved": it differs from the run version -- save and run, run that
 *   version, or not.
 */
export type RunGate = "run" | "askSave" | "askUnsaved" | "settle";

export function runGate(doc: PlanDoc): RunGate {
  if (doc.changes.length > 0) return "settle";
  if (!unsaved(doc)) return "run";
  return doc.saved?.length ? "askUnsaved" : "askSave";
}

/** The fields of a step a drawer's text boxes hold, as typed. */
export type StepDraft = { title: string; instruction: string; inputs: string; outputs: string; done_when: string };

/** One item a line, as the boxes say ("one a line"): a comma is part of an item. */
const lines = (v: string) =>
  v
    .split("\n")
    .map((x) => x.trim())
    .filter(Boolean);

/**
 * The edit a drawer's text boxes make to a step: only the fields that
 * changed (an empty title keeps the old one), and of those only the ones
 * in `touched` when it is given -- a box the human did not type in holds
 * what the step said when it was filled, and must not write that back over
 * a change made elsewhere meanwhile.
 */
export function editedFields(step: PlanStep, d: StepDraft, touched?: ReadonlySet<keyof StepDraft>): Record<string, unknown> {
  const all = editedAll(step, d);
  if (!touched) return all;
  return Object.fromEntries(Object.entries(all).filter(([k]) => touched.has(k as keyof StepDraft)));
}

function editedAll(step: PlanStep, d: StepDraft): Record<string, unknown> {
  const out: Record<string, unknown> = {};
  const title = d.title.trim();
  if (title && title !== step.title) out.title = title;
  if (d.instruction.trim() !== step.instruction) out.instruction = d.instruction.trim();
  if (d.done_when.trim() !== step.done_when) out.done_when = d.done_when.trim();
  const inputs = lines(d.inputs);
  if (JSON.stringify(inputs) !== JSON.stringify(step.inputs)) out.inputs = inputs;
  const outputs = lines(d.outputs);
  if (JSON.stringify(outputs) !== JSON.stringify(step.outputs)) out.outputs = outputs;
  return out;
}

/**
 * Whether opening a routine's plan should ask its agent for a first draft:
 * only when the routine had no plan before this press, the plan is still
 * empty (no steps, no suggestions), and no draft was asked for it already
 * and none is under way. Pressing again, or coming back to the page while
 * the agent works, asks nothing more.
 */
export function needsFirstDraft(o: { hadPlan: boolean; doc: PlanDoc; asked: boolean; busy: boolean }): boolean {
  return !o.hadPlan && !o.asked && !o.busy && o.doc.steps.length === 0 && o.doc.changes.length === 0;
}

/** The saved version the working plan is, if any (the newest of equals). */
export function sameVersion(doc: PlanDoc): number | undefined {
  const now = content(doc.steps);
  return [...(doc.saved ?? [])].reverse().find((v) => content(v.steps) === now)?.n;
}

/**
 * Whether opening version `n` should ask first: it would replace a working
 * plan that is in no saved version (changes that would be lost). Opening
 * the version the working plan already is asks nothing.
 */
export function openNeedsConfirm(doc: PlanDoc, n: number): boolean {
  const same = sameVersion(doc);
  if (same === n) return false;
  return same === undefined && doc.steps.length > 0;
}

/** Runs read per page of the history menu. */
export const RUNS_PAGE = 20;

/** The next limit to read the history with, or null when everything is in (a page came back short). */
export function nextRunsLimit(loaded: number, limit: number, page = RUNS_PAGE): number | null {
  return loaded < limit ? null : limit + page;
}

/** Whether the routine's instruction moved on since the plan was made. */
export function instructionChanged(doc: PlanDoc, instruction: string): boolean {
  return !!doc.source && doc.source.instruction.trim() !== instruction.trim();
}

/** A run's step results by step, read from what the core kept. */
export function resultsOf(steps: unknown): Map<string, StepResult> {
  const out = new Map<string, StepResult>();
  if (!Array.isArray(steps)) return out;
  for (const r of steps) {
    if (!r || typeof r !== "object") continue;
    const { id, status, note } = r as Record<string, unknown>;
    if (typeof id !== "string") continue;
    if (status !== "done" && status !== "failed" && status !== "skipped") continue;
    out.set(id, { id, status, note: typeof note === "string" ? note : undefined });
  }
  return out;
}

/** The glyph of one step's result: ✓ done, ✗ failed, – skipped, · nothing said of it. */
export const RESULT_GLYPH = { done: "✓", failed: "✗", skipped: "–", none: "·" } as const;

/** The one-line glyphs of a run's step results, in plan order. */
export function resultGlyphs(order: string[], results: Map<string, StepResult>): string {
  return order.map((id) => RESULT_GLYPH[results.get(id)?.status ?? "none"]).join("");
}

/** A step as a run was given it: every word of it from the core; titles and waits at least. */
export type GivenStep = Partial<PlanStep> & { id: string; title: string; after: string[] };

/** Mirrors `routine::live`: a run of the routine in flight. */
export type LiveRun = {
  run: string;
  /** The plan it was given: its run version (null when it was given none). */
  plan: { version: number; steps: GivenStep[] } | null;
  /** What its agent said so far of each step, through `plan_step`. */
  progress: Record<string, { status: "started" | "done" | "failed" | "skipped"; note: string }>;
};

/** Where a step stands in a run in flight. */
export type RunState = "running" | "done" | "failed" | "skipped" | "waiting";

/**
 * The plan as a run in flight shows it. The run goes by the version it was
 * given, so the graph does too: its steps with their titles and waits as
 * given, the rest of each step's words from that saved version (`saved`,
 * when it is still there unchanged in id) or else the working plan, each
 * with where the agent said it is. Edits made meanwhile wait for the next
 * run.
 *
 * A step the agent has said nothing of is under way once every step it
 * waits for is done (or skipped), and waiting before that: so the first
 * wave shows as running the moment the run starts, before the agent says
 * a word, and what it says (plan_step) takes over step by step. `told`
 * says whether it has said anything yet.
 */
export function runView(
  working: PlanStep[],
  live: LiveRun | null,
  saved: PlanVersion[] = [],
): { steps: PlanStep[]; states: Map<string, RunState>; told: boolean } | null {
  if (!live?.plan) return null;
  const given = saved.find((v) => v.n === live.plan!.version)?.steps ?? [];
  const byId = new Map([...working, ...given].map((s) => [s.id, s]));
  const blank: PlanStep = { id: "", title: "", instruction: "", after: [], inputs: [], outputs: [], done_when: "", agent: "", on_fail: "stop", by: "" };
  // The run's own words (all of them, from a run that kept them; else its
  // title and waits over the saved version's), whatever the working plan says now.
  const steps: PlanStep[] = live.plan.steps.map((g) => ({ ...blank, ...byId.get(g.id), ...g, x: undefined, y: undefined }));
  const told = Object.keys(live.progress ?? {}).length > 0;
  const said = (id: string) => live.progress?.[id]?.status;
  const ids = new Set(steps.map((s) => s.id));
  const states = new Map<string, RunState>();
  for (const s of steps) {
    const p = said(s.id);
    if (p === "started") states.set(s.id, "running");
    else if (p === "done" || p === "failed" || p === "skipped") states.set(s.id, p);
    else {
      const ready = s.after.filter((a) => ids.has(a)).every((a) => said(a) === "done" || said(a) === "skipped");
      states.set(s.id, ready ? "running" : "waiting");
    }
  }
  return { steps, states, told };
}

/**
 * The run in flight as the window can say it the moment Run is pressed,
 * before the core has opened the agent: the plan's run version (what the
 * run will be given), or none when nothing is saved. The core's own state
 * (routine_live) replaces it as soon as it has one.
 */
export function pressedRun(doc: PlanDoc): LiveRun {
  const given = runVersion(doc);
  return {
    run: "",
    plan: given && given.steps.length ? { version: given.n, steps: given.steps.map(({ x: _x, y: _y, ...s }) => s) } : null,
    progress: {},
  };
}

/** How far a step has got, for merging what two reports said of it. */
const STAGE = { started: 1, done: 2, failed: 2, skipped: 2 } as const;

/**
 * The run in flight after the core's account of it (routine_live) arrives.
 * Progress events can reach the window before an account read earlier, so
 * for the same run each step keeps the furthest word either said; another
 * run's account replaces this one outright.
 */
export function mergeLive(had: LiveRun | null, got: LiveRun | null): LiveRun | null {
  if (!got || !had || (had.run && had.run !== got.run)) return got;
  const progress = { ...got.progress };
  for (const [id, p] of Object.entries(had.progress)) {
    const theirs = progress[id];
    if (!theirs || STAGE[p.status] > STAGE[theirs.status]) progress[id] = p;
  }
  return { ...got, progress };
}
