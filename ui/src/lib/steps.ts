/**
 * The steps a turn takes on its way to an answer: the tool calls and the
 * thinking between them, shown as a list that grows while the turn works
 * and folds into one summary line once the answer starts.
 *
 * Everything here is pure so `node --test` can reach it; the drawing is
 * StepGroup.svelte. The types are structural rather than imported from the
 * store, which pulls in Tauri.
 */

/** The part of `store.Tool` a step needs. */
export type StepTool = {
  id: string;
  title: string;
  toolKind: string;
  status: string;
  /** When the call began and ended, in the run's milliseconds. */
  startedMs?: number;
  endedMs?: number;
};

/** The part of `store.Segment` a step needs. */
export type StepSegment = { kind: "text"; text: string } | { kind: "thought"; text: string } | { kind: "tool"; tool: StepTool };

export type Step = { kind: "tool"; tool: StepTool } | { kind: "thought"; text: string };

/**
 * What a turn shows, in order: prose, and the steps between prose. `at` is
 * the block's place among the steps blocks of its turn, which stays put as
 * the turn grows, so it can key what the human folded or unfolded.
 */
export type Block = { kind: "text"; text: string } | { kind: "steps"; steps: Step[]; at: number };

/**
 * Consecutive tool calls and thoughts become one steps block; they used to
 * overwrite one line the way a terminal does with a carriage return. Text
 * that `keep` says is empty (hook chatter, whitespace) is dropped without
 * splitting the steps around it, and so is an empty thought.
 */
export function shape(segments: StepSegment[], keep: (text: string) => boolean = (s) => !!s.trim()): Block[] {
  const out: Block[] = [];
  let groups = 0;
  const steps = (): Step[] => {
    const last = out.at(-1);
    if (last?.kind === "steps") return last.steps;
    const block: Block = { kind: "steps", steps: [], at: groups++ };
    out.push(block);
    return block.steps;
  };
  for (const seg of segments) {
    if (seg.kind === "tool") steps().push({ kind: "tool", tool: seg.tool });
    else if (seg.kind === "thought") {
      if (seg.text.trim()) steps().push({ kind: "thought", text: seg.text.trim() });
    } else if (keep(seg.text)) out.push({ kind: "text", text: seg.text });
  }
  return out;
}

/** The steps block still being added to: the last thing a live turn shows. */
export function isActive(blocks: Block[], i: number, live: boolean): boolean {
  return live && i === blocks.length - 1 && blocks[i]?.kind === "steps";
}

/**
 * Open while it is being added to, folded once the turn has moved on, and
 * whatever the human chose after they chose it.
 */
export function isOpen(active: boolean, chosen: boolean | undefined): boolean {
  return chosen ?? active;
}

/** How many steps a long list shows before the rest fold away above it. */
export const RECENT = 6;

/**
 * The newest `RECENT` steps, and how many earlier ones are hidden, so a
 * turn of eighty calls does not push its answer off the screen.
 */
export function visible(steps: Step[], all: boolean, recent = RECENT): { hidden: number; shown: Step[] } {
  const hidden = all ? 0 : Math.max(0, steps.length - recent);
  return { hidden, shown: steps.slice(hidden) };
}

/** Still going: neither finished nor failed. */
export function running(tool: StepTool): boolean {
  return tool.status !== "completed" && tool.status !== "failed";
}

/** The kinds that have their own verb; anything else is `other`. */
const KINDS = ["read", "edit", "delete", "move", "search", "execute", "think", "fetch", "switchmode"];

/**
 * The i18n key of a step's verb: "Reading" while it runs in a live turn,
 * "Read" once it is over (or the turn is, which ends it either way).
 */
export function verbKey(step: Step, live: boolean): string {
  const kind = step.kind === "thought" ? "think" : KINDS.includes(step.tool.toolKind) ? step.tool.toolKind : "other";
  const now = step.kind === "tool" && live && running(step.tool);
  return `steps.verb.${kind}.${now ? "now" : "done"}`;
}

/** Leading words an agent puts in a title that the verb already says. */
const SAID = /^(?:Read|Edit|Write|Find|Fetch|Search|Delete|Move)\s+/;

/**
 * What the step was about, without what the verb says: the file, the
 * query, the command. MCP tools arrive as `mcp__divixi__spawn_worker`, and
 * a command comes wrapped in backticks.
 */
export function detail(tool: StepTool): string {
  let s = tool.title.trim().replace(/^mcp__[a-z0-9_-]+__/i, "").replace(/^mcp\.[a-z0-9_-]+\./i, "");
  s = s.replace(SAID, "");
  const fenced = /^`([^`]+)`$/.exec(s);
  return fenced ? fenced[1] : s;
}

/** "840ms", "1.2s", "2m 5s". */
export function span(ms: number): string {
  if (ms < 1000) return `${Math.round(ms)}ms`;
  if (ms < 60_000) return `${(ms / 1000).toFixed(1)}s`;
  const s = Math.round(ms / 1000);
  return `${Math.floor(s / 60)}m ${s % 60}s`;
}

/** How long one call took, once it is over and both ends were seen. */
export function took(tool: StepTool): number | undefined {
  if (running(tool) || tool.startedMs === undefined || tool.endedMs === undefined) return undefined;
  return Math.max(0, tool.endedMs - tool.startedMs);
}

/** What the folded line says: calls, failures, and the time from first to last. */
export function summarize(steps: Step[]): { tools: number; failed: number; ms?: number } {
  let tools = 0;
  let failed = 0;
  let first: number | undefined;
  let last: number | undefined;
  for (const s of steps) {
    if (s.kind !== "tool") continue;
    tools++;
    if (s.tool.status === "failed") failed++;
    if (s.tool.startedMs !== undefined) first = Math.min(first ?? Infinity, s.tool.startedMs);
    if (s.tool.endedMs !== undefined) last = Math.max(last ?? -Infinity, s.tool.endedMs);
  }
  const ms = first !== undefined && last !== undefined && last >= first ? last - first : undefined;
  return { tools, failed, ms };
}
