import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

/** Mirrors `orchestra_core::LaneEvent` — serde tags it with `kind`. */
export type LaneEvent =
  | { kind: "connected"; protocol: string; load_session: boolean }
  | { kind: "started"; session_id: string; cwd: string }
  | { kind: "message"; text: string }
  | { kind: "thought"; text: string }
  | { kind: "tool_call"; id: string; title: string; tool_kind: string; status: string }
  | { kind: "tool_update"; id: string; status: string }
  | { kind: "plan"; entries: string[] }
  | { kind: "usage"; raw: unknown }
  | { kind: "finished"; stop_reason: string }
  | { kind: "failed"; error: string };

export type Envelope = {
  lane: string;
  run: string;
  at_ms: number;
  event: LaneEvent;
};

/** Mirrors `orchestra_store::RunSummary`: one run as the timeline sees it. */
export type RunSummary = {
  id: string;
  lane: string;
  prompt: string;
  cwd: string;
  status: RunStatus;
  started_at: number;
  duration_ms: number | null;
  session_id: string | null;
  stop_reason: string | null;
  error: string | null;
  output: string;
  plan: string[];
  tool_count: number;
};

/** Mirrors `orchestra_store::StoredEvent`. */
export type StoredEvent = { seq: number; at_ms: number; event: LaneEvent };

export type Tool = { id: string; title: string; toolKind: string; status: string };

export type TranscriptLine = { ms: number; label: string; text: string; tone: Tone };
export type Tone = "in" | "out" | "ok" | "warn" | "dim";

export type RunStatus = "connecting" | "running" | "done" | "failed";

export type Run = {
  id: string;
  lane: string;
  prompt: string;
  status: RunStatus;
  sessionId?: string;
  cwd?: string;
  /** Unix milliseconds. */
  startedAt: number;
  durationMs?: number;
  /** Above the membrane: what the Report card shows. */
  message: string;
  plan: string[];
  toolCount: number;
  stopReason?: string;
  error?: string;
  /**
   * Below the membrane: what the inspector shows. Empty until `loaded`,
   * which is immediate for live runs and lazy for restored ones.
   */
  loaded: boolean;
  thought: string;
  tools: Tool[];
  transcript: TranscriptLine[];
};

/** Everything above the membrane plus, per run, the detail kept below it. */
class Store {
  theme = $state<"dk" | "lt">("dk");
  runs = $state<Run[]>([]);
  /** Run id whose lane detail is open in the inspector; "" means closed. */
  inspecting = $state("");
  busy = $state(false);
  lastError = $state("");
  restored = $state(false);

  get openRun(): Run | undefined {
    return this.runs.find((r) => r.id === this.inspecting);
  }

  get activeRun(): Run | undefined {
    return this.runs.find((r) => r.status === "running" || r.status === "connecting");
  }

  toggleTheme() {
    this.theme = this.theme === "dk" ? "lt" : "dk";
    document.documentElement.className = this.theme;
  }

  /** Rebuild the timeline from the store. Called once at startup. */
  async restore() {
    try {
      const summaries = await invoke<RunSummary[]>("list_runs");
      this.runs = summaries.map(fromSummary);
      // The core closes runs left live by a previous process, so nothing
      // restored can be in flight.
      this.busy = false;
    } catch (err) {
      this.lastError = String(err);
    } finally {
      this.restored = true;
    }
  }

  /** Open (or close) a run in the inspector, loading its lane detail on demand. */
  async inspect(id: string) {
    if (this.inspecting === id) {
      this.inspecting = "";
      return;
    }
    this.inspecting = id;
    const run = this.runs.find((r) => r.id === id);
    if (run && !run.loaded) await this.hydrate(run);
  }

  /** Replay a run's event log into its below-membrane fields. */
  async hydrate(run: Run) {
    try {
      const events = await invoke<StoredEvent[]>("run_events", { run: run.id });
      // Replaying folds the message back together; start from empty so the
      // summary text is not doubled.
      run.message = "";
      run.thought = "";
      run.tools = [];
      run.plan = [];
      run.transcript = [];
      for (const e of events) fold(run, e.at_ms, e.event);
      run.loaded = true;
    } catch (err) {
      this.lastError = String(err);
    }
  }

  /** Ask the core to open a lane and run one prompt. */
  async send(prompt: string, lane = "solo") {
    const text = prompt.trim();
    if (!text || this.busy) return;
    this.busy = true;
    this.lastError = "";
    try {
      const id = await invoke<string>("start_run", { lane, prompt: text });
      this.runs.push({
        id,
        lane,
        prompt: text,
        status: "connecting",
        startedAt: Date.now(),
        message: "",
        plan: [],
        toolCount: 0,
        loaded: true,
        thought: "",
        tools: [],
        transcript: [],
      });
    } catch (err) {
      this.busy = false;
      this.lastError = String(err);
    }
  }

  /** Fold one live lane event into the run it belongs to. */
  apply(env: Envelope) {
    const run = this.runs.find((r) => r.id === env.run);
    if (!run) return;
    fold(run, env.at_ms, env.event);
    if (env.event.kind === "finished" || env.event.kind === "failed") {
      this.busy = false;
    }
  }
}

/** Pure state transition: the same fold serves live events and replay. */
function fold(run: Run, ms: number, ev: LaneEvent) {
  switch (ev.kind) {
    case "connected":
      push(run, ms, "connect", `protocol ${ev.protocol} · loadSession=${ev.load_session}`, "ok");
      break;
    case "started":
      run.status = "running";
      run.sessionId = ev.session_id;
      run.cwd = ev.cwd;
      push(run, ms, "session", `${ev.session_id}  cwd=${ev.cwd}`, "out");
      break;
    case "message":
      run.message += ev.text;
      break;
    case "thought":
      run.thought += ev.text;
      break;
    case "tool_call":
      run.tools.push({ id: ev.id, title: ev.title, toolKind: ev.tool_kind, status: ev.status });
      run.toolCount = run.tools.length;
      push(run, ms, ev.tool_kind, ev.title, "dim");
      break;
    case "tool_update": {
      const tool = run.tools.find((t) => t.id === ev.id);
      if (tool) tool.status = ev.status;
      break;
    }
    case "plan":
      run.plan = ev.entries;
      push(run, ms, "plan", `${ev.entries.length} entries`, "dim");
      break;
    case "usage":
      break;
    case "finished":
      run.status = "done";
      run.stopReason = ev.stop_reason;
      run.durationMs = ms;
      push(run, ms, "done", ev.stop_reason, "ok");
      break;
    case "failed":
      run.status = "failed";
      run.error = ev.error;
      run.durationMs = ms;
      push(run, ms, "error", ev.error, "warn");
      break;
  }
}

function fromSummary(s: RunSummary): Run {
  return {
    id: s.id,
    lane: s.lane,
    prompt: s.prompt,
    status: s.status,
    sessionId: s.session_id ?? undefined,
    cwd: s.cwd,
    startedAt: s.started_at,
    durationMs: s.duration_ms ?? undefined,
    message: s.output,
    plan: s.plan,
    toolCount: s.tool_count,
    stopReason: s.stop_reason ?? undefined,
    error: s.error ?? undefined,
    loaded: false,
    thought: "",
    tools: [],
    transcript: [],
  };
}

function push(run: Run, ms: number, label: string, text: string, tone: Tone) {
  run.transcript.push({ ms, label, text, tone });
}

export const store = new Store();

/** Subscribe once; the core emits one `lane` event per lane event. */
export async function connectEvents() {
  await listen<Envelope>("lane", (e) => store.apply(e.payload));
}
