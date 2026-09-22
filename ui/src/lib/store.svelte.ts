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
  agent: string;
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

/** Mirrors `orchestra_agents::AgentKind` ids. */
export type AgentId = "claude_code" | "codex" | "copilot" | "antigravity";

export type Readiness = "ready" | "needs_login" | "needs_download" | "not_installed" | "error";

export type AuthMethodInfo = {
  id: string;
  name: string;
  description: string | null;
  terminal_command: string | null;
};

/** Mirrors `orchestra_agents::AgentStatus`. */
export type AgentStatus = {
  kind: AgentId;
  name: string;
  readiness: Readiness;
  cli: { path: string; version: string | null } | null;
  adapter: { kind: string; path?: string; package?: string; reason?: string };
  probe: {
    protocol: string;
    agent_name: string | null;
    agent_version: string | null;
    auth_methods: AuthMethodInfo[];
  } | null;
  error: string | null;
  login_hint: string;
  install_hint: string;
};

export type Tool = { id: string; title: string; toolKind: string; status: string };

export type TranscriptLine = { ms: number; label: string; text: string; tone: Tone };
export type Tone = "in" | "out" | "ok" | "warn" | "dim";

export type RunStatus = "connecting" | "running" | "done" | "failed";

export type Run = {
  id: string;
  lane: string;
  agent: string;
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

export type View = "track" | "settings";

/** Mirrors the `agent_download` event payload. */
export type DownloadProgress = {
  agent: AgentId;
  phase: "downloading" | "unpacking" | "done";
  received: number;
  total: number | null;
};

/** Everything above the membrane plus, per run, the detail kept below it. */
class Store {
  theme = $state<"dk" | "lt">("dk");
  view = $state<View>("track");
  /** First-run agent setup, shown in front of the shell. */
  setupOpen = $state(false);
  runs = $state<Run[]>([]);
  /** Run id whose lane detail is open in the inspector; "" means closed. */
  inspecting = $state("");
  busy = $state(false);
  lastError = $state("");
  restored = $state(false);

  /** Last detection result; null until setup has run once. */
  agents = $state<AgentStatus[] | null>(null);
  /** Agent id lanes are opened on. */
  agent = $state<AgentId>("claude_code");
  detecting = $state(false);
  /** Agent ids with a login or download in flight. */
  working = $state<Record<string, string>>({});
  /** Live download progress per agent id, while a download runs. */
  downloads = $state<Record<string, DownloadProgress>>({});

  get openRun(): Run | undefined {
    return this.runs.find((r) => r.id === this.inspecting);
  }

  get activeRun(): Run | undefined {
    return this.runs.find((r) => r.status === "running" || r.status === "connecting");
  }

  get readyAgents(): AgentStatus[] {
    return (this.agents ?? []).filter((a) => a.readiness === "ready");
  }

  get currentAgent(): AgentStatus | undefined {
    return this.agents?.find((a) => a.kind === this.agent);
  }

  toggleTheme() {
    this.theme = this.theme === "dk" ? "lt" : "dk";
    document.documentElement.className = this.theme;
  }

  /** Rebuild the timeline and agent list from the store. Called once at startup. */
  async restore() {
    try {
      const [summaries, agents] = await Promise.all([
        invoke<RunSummary[]>("list_runs"),
        invoke<AgentStatus[] | null>("agent_statuses"),
      ]);
      this.runs = summaries.map(fromSummary);
      // The core closes runs left live by a previous process, so nothing
      // restored can be in flight.
      this.busy = false;
      this.agents = agents;
      this.pickDefaultAgent();
      // Setup comes first when nothing has been detected yet, or when the
      // last detection left nothing to run lanes on.
      if (agents === null || this.readyAgents.length === 0) this.setupOpen = true;
    } catch (err) {
      this.lastError = String(err);
    } finally {
      this.restored = true;
    }
  }

  /** Keep the selected agent on one that is ready. */
  private pickDefaultAgent() {
    const ready = this.readyAgents;
    if (ready.some((a) => a.kind === this.agent)) return;
    if (ready.length) this.agent = ready[0].kind;
  }

  /** Launch and probe every agent. Several seconds. */
  async detect() {
    if (this.detecting) return;
    this.detecting = true;
    this.lastError = "";
    try {
      this.agents = await invoke<AgentStatus[]>("detect_agents");
      this.pickDefaultAgent();
    } catch (err) {
      this.lastError = String(err);
    } finally {
      this.detecting = false;
    }
  }

  /** Run the agent's own ACP login flow. May open a browser. */
  async login(agent: AgentId, method?: string) {
    await this.workOn(agent, "로그인 중", () => invoke<AgentStatus>("login_agent", { agent, method: method ?? null }));
  }

  /** Fetch the agent's ACP server (Antigravity). Progress arrives as events. */
  async download(agent: AgentId) {
    this.downloads = { ...this.downloads, [agent]: { agent, phase: "downloading", received: 0, total: null } };
    try {
      await this.workOn(agent, "다운로드 중", () => invoke<AgentStatus>("download_agent", { agent }));
    } finally {
      const { [agent]: _done, ...rest } = this.downloads;
      this.downloads = rest;
    }
  }

  /** Fold a progress event in. */
  progress(p: DownloadProgress) {
    if (!(p.agent in this.downloads)) return;
    this.downloads = { ...this.downloads, [p.agent]: p };
  }

  private async workOn(agent: AgentId, label: string, op: () => Promise<AgentStatus>) {
    if (this.working[agent]) return;
    this.working = { ...this.working, [agent]: label };
    this.lastError = "";
    try {
      const next = await op();
      this.agents = (this.agents ?? []).map((a) => (a.kind === agent ? next : a));
      if (!this.agents.some((a) => a.kind === agent)) this.agents = [...this.agents, next];
      this.pickDefaultAgent();
    } catch (err) {
      this.lastError = String(err);
    } finally {
      const { [agent]: _done, ...rest } = this.working;
      this.working = rest;
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

  /** Ask the core to open a lane on the selected agent and run one prompt. */
  async send(prompt: string, lane = "solo") {
    const text = prompt.trim();
    if (!text || this.busy) return;
    this.busy = true;
    this.lastError = "";
    const agent = this.agent;
    try {
      const id = await invoke<string>("start_run", { lane, prompt: text, agent });
      this.runs.push({
        id,
        lane,
        agent,
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
    agent: s.agent,
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

/** Short mono label for an agent id. */
export function agentLabel(id: string): string {
  switch (id) {
    case "claude_code":
      return "claude-code";
    case "codex":
      return "codex";
    case "copilot":
      return "copilot";
    case "antigravity":
      return "antigravity";
    default:
      return id;
  }
}

export const store = new Store();

/** Subscribe once; the core emits one `lane` event per lane event. */
export async function connectEvents() {
  await Promise.all([
    listen<Envelope>("lane", (e) => store.apply(e.payload)),
    listen<DownloadProgress>("agent_download", (e) => store.progress(e.payload)),
  ]);
}
