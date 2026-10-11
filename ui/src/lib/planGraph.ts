/**
 * A routine's plan as the window reads it: the shapes the core sends
 * (src-tauri/src/plan.rs), and what the screens make of them — the waves,
 * where each step sits on the graph, which steps a step may wait for, and
 * what the agent's suggestions did. Kept apart from the components so it is
 * tested.
 */

export type Approval = "none" | "ask";
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
  approval: Approval;
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
  use_in_run: boolean;
  proposal?: { text: string; run: string | null; at: number } | null;
  /** Saved versions, oldest first; runs are given the last. */
  saved?: PlanVersion[];
};

/** Mirrors `plan::Version`. */
export type PlanVersion = { n: number; at: number; note: string; steps: PlanStep[] };

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
  use_in_run: true,
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
  const out = new Map(auto);
  const placedX = (id: string) => out.get(id)!.x;
  for (const w of ws) {
    for (const id of w) {
      const s = index.get(id)!;
      if (s.x === undefined || s.y === undefined) continue;
      const before = s.after.filter((a) => out.has(a));
      const afterIt = steps.filter((o) => o.after.includes(id)).map((o) => o.id);
      const rightOfBefore = before.every((a) => s.x! > placedX(a));
      const leftOfAfter = afterIt.every((o) => s.x! < (index.get(o)!.x ?? auto.get(o)!.x));
      if (rightOfBefore && leftOfAfter) out.set(id, { x: s.x, y: s.y });
    }
  }
  return out;
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
    const kind: SuggestionKind = !c.before ? "added" : !c.after ? "removed" : "changed";
    out.set(c.target, { change: c, kind });
  }
  return out;
}

/** What a changed step's suggestion changed, by field, for the card to say. */
export function changedFields(c: PlanChange): string[] {
  if (!c.before || !c.after) return [];
  const fields: (keyof PlanStep)[] = ["title", "instruction", "after", "inputs", "outputs", "done_when", "approval", "on_fail", "agent"];
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

/** Whether the working plan differs from the last saved version (a plan never saved, once it has steps). */
export function unsaved(doc: PlanDoc): boolean {
  const last = doc.saved?.at(-1);
  if (!last) return doc.steps.length > 0;
  return content(last.steps) !== content(doc.steps);
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

/** The one-line glyphs of a run's step results, in plan order: ✓ ✗ –. */
export function resultGlyphs(order: string[], results: Map<string, StepResult>): string {
  return order.map((id) => ({ done: "✓", failed: "✗", skipped: "–" })[results.get(id)?.status ?? "skipped"]).join("");
}

/** Mirrors `routine::live`: a run of the routine in flight. */
export type LiveRun = {
  run: string;
  /** The plan it was given: its saved version (null when it was given none). */
  plan: { version: number; steps: { id: string; title: string; after: string[] }[] } | null;
  /** What its agent said so far of each step, through `plan_step`. */
  progress: Record<string, { status: "started" | "done" | "failed" | "skipped"; note: string }>;
};

/** Where a step stands in a run in flight. */
export type RunState = "running" | "done" | "failed" | "skipped" | "waiting";

/**
 * The plan as a run in flight shows it. The run goes by the version it was
 * given, so the graph does too: its steps (with the working plan's words
 * where a step is still there, its own title where not), each with where
 * the agent said it is. Edits made meanwhile wait for the next run.
 *
 * `told` is whether the agent has said anything yet: until it does, steps
 * are not dimmed as "waiting" -- all that is known is that the run is on.
 */
export function runView(working: PlanStep[], live: LiveRun | null): { steps: PlanStep[]; states: Map<string, RunState>; told: boolean } | null {
  if (!live?.plan) return null;
  const byId = new Map(working.map((s) => [s.id, s]));
  const steps: PlanStep[] = live.plan.steps.map((g) => {
    const w = byId.get(g.id);
    // The run's own order of steps (its waits), whatever the working plan says now.
    return w ? { ...w, title: w.title, after: g.after, x: undefined, y: undefined } : {
      id: g.id, title: g.title, instruction: "", after: g.after, inputs: [], outputs: [], done_when: "", agent: "", approval: "none", on_fail: "stop", by: "",
    };
  });
  const told = Object.keys(live.progress ?? {}).length > 0;
  const states = new Map<string, RunState>();
  for (const s of steps) {
    const p = live.progress?.[s.id]?.status;
    states.set(s.id, p === "started" ? "running" : p === "done" || p === "failed" || p === "skipped" ? p : "waiting");
  }
  return { steps, states, told };
}
