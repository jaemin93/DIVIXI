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
  startedAt: number;
  durationMs?: number;
  message: string;
  thought: string;
  tools: Tool[];
  plan: string[];
  transcript: TranscriptLine[];
  stopReason?: string;
  error?: string;
};

/** Everything above the membrane plus, per run, the detail kept below it. */
class Store {
  theme = $state<"dk" | "lt">("dk");
  runs = $state<Run[]>([]);
  /** Run id whose lane detail is open in the inspector; "" means closed. */
  inspecting = $state("");
  busy = $state(false);
  lastError = $state("");

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
        thought: "",
        tools: [],
        plan: [],
        transcript: [],
      });
    } catch (err) {
      this.busy = false;
      this.lastError = String(err);
    }
  }

  /** Fold one lane event into the run it belongs to. */
  apply(env: Envelope) {
    const run = this.runs.find((r) => r.id === env.run);
    if (!run) return;
    const ev = env.event;

    switch (ev.kind) {
      case "connected":
        push(run, env.at_ms, "connect", `protocol ${ev.protocol} · loadSession=${ev.load_session}`, "ok");
        break;
      case "started":
        run.status = "running";
        run.sessionId = ev.session_id;
        run.cwd = ev.cwd;
        push(run, env.at_ms, "session", `${ev.session_id}  cwd=${ev.cwd}`, "out");
        break;
      case "message":
        run.message += ev.text;
        break;
      case "thought":
        run.thought += ev.text;
        break;
      case "tool_call":
        run.tools.push({ id: ev.id, title: ev.title, toolKind: ev.tool_kind, status: ev.status });
        push(run, env.at_ms, ev.tool_kind, ev.title, "dim");
        break;
      case "tool_update": {
        const tool = run.tools.find((t) => t.id === ev.id);
        if (tool) tool.status = ev.status;
        break;
      }
      case "plan":
        run.plan = ev.entries;
        push(run, env.at_ms, "plan", `${ev.entries.length} entries`, "dim");
        break;
      case "usage":
        break;
      case "finished":
        run.status = "done";
        run.stopReason = ev.stop_reason;
        run.durationMs = env.at_ms;
        push(run, env.at_ms, "done", ev.stop_reason, "ok");
        this.busy = false;
        break;
      case "failed":
        run.status = "failed";
        run.error = ev.error;
        run.durationMs = env.at_ms;
        push(run, env.at_ms, "error", ev.error, "warn");
        this.busy = false;
        break;
    }
  }
}

function push(run: Run, ms: number, label: string, text: string, tone: Tone) {
  run.transcript.push({ ms, label, text, tone });
}

export const store = new Store();

/** Subscribe once; the core emits one `lane` event per lane event. */
export async function connectEvents() {
  await listen<Envelope>("lane", (e) => store.apply(e.payload));
}
