import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { boardPng, briefOf } from "./ink";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { i18n, systemLang, t, type Lang, type LangPref } from "./i18n.svelte";

/** Mirrors `orchestra_core::AgentEvent` — serde tags it with `kind`. */
export type AgentEvent =
  | { kind: "connected"; protocol: string; load_session: boolean }
  | { kind: "started"; session_id: string; cwd: string }
  | { kind: "message"; text: string }
  | { kind: "thought"; text: string }
  | { kind: "tool_call"; id: string; title: string; tool_kind: string; status: string }
  | { kind: "tool_update"; id: string; status: string }
  | { kind: "plan"; entries: string[] }
  | { kind: "usage"; raw: unknown }
  | { kind: "commands"; commands: SlashCommand[] }
  | {
      kind: "permission";
      request: string;
      title: string;
      tool_kind: string;
      input: string;
      options: { id: string; name: string; kind: string }[];
    }
  | { kind: "finished"; stop_reason: string }
  | { kind: "failed"; error: string };

/** Mirrors `orchestra_core::SlashCommand`: one `/name` the agent offers. */
export type SlashCommand = { name: string; description: string; hint: string | null };

/** Mirrors `conductor::ConductorState`. */
export type ConductorState = { open: boolean; busy: boolean; agent: string | null; commands: SlashCommand[] };

export type Envelope = {
  track: string;
  session: string;
  run: string;
  at_ms: number;
  event: AgentEvent;
};

/** Session options chosen for one role: `option id → value id`, in the agent's own terms. */
export type OptionConfig = Record<string, string>;

/** Mirrors `orchestra_store::TrackInfo`: one conductor, its workers, one folder. */
export type Track = {
  id: string;
  name: string;
  intent: string;
  cwd: string;
  /** Agent the conductor runs on. */
  agent: string;
  conductor_config: OptionConfig;
  /** Agent workers run on; empty means the conductor's. */
  worker_agent: string;
  worker_config: OptionConfig;
  /** `#rrggbb` for the list, or empty. */
  color: string;
  tags: string[];
  created_at: number;
  updated_at: number;
  runs: number;
};

/** Mirrors `orchestra_store::TrackPatch`: fields to set, the rest kept. */
export type TrackPatch = Partial<{
  name: string;
  intent: string;
  cwd: string;
  agent: string;
  conductor_config: OptionConfig;
  worker_agent: string;
  worker_config: OptionConfig;
  color: string;
  tags: string[];
}>;

/** How the track list is narrowed, ordered and folded. Persisted as the `tracks_filter` setting. */
export type TrackSort = "recent" | "oldest" | "created-desc" | "created-asc" | "az" | "za";
export type TrackFilter = {
  /** Only tracks with a run in flight. */
  running: boolean;
  /** Only tracks whose conductor session is open. */
  active: boolean;
  /** Only tracks with activity inside this window; empty means any. */
  recent: "" | "1h" | "24h" | "7d";
  sort: TrackSort;
  /** Tracks idle longer than this many days fold away; 0 keeps them all in place. */
  fold: number;
  /** Only tracks carrying every one of these tags. */
  tags: string[];
};
export const DEFAULT_TRACK_FILTER: TrackFilter = { running: false, active: false, recent: "", sort: "recent", fold: 7, tags: [] };

/** A tag the user made: its name and the colour it was dealt. */
export type TagDef = { name: string; color: string };

/** Colours a track can carry in the list; muted enough for both themes. */
export const TRACK_COLORS = ["#7aa2f7", "#73daca", "#9ece6a", "#e0af68", "#ff9e64", "#f7768e", "#bb9af7", "#c0caf5"];

/** The agent a track's workers run on. */
export function workerAgentOf(track: Track): string {
  return track.worker_agent || track.agent;
}

/** The options workers open with: the worker's own, else (never set apart) the conductor's. */
export function workerConfigOf(track: Track): OptionConfig {
  const own = track.worker_agent !== "" || Object.keys(track.worker_config).length > 0;
  return own ? track.worker_config : track.conductor_config;
}

/** Who a session setting is for. */
export type Role = "conductor" | "worker";

/** Mirrors `orchestra_store::RunSummary`: one run as the timeline sees it. */
export type RunSummary = {
  id: string;
  track: string;
  session: string;
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
export type StoredEvent = { seq: number; at_ms: number; event: AgentEvent };

/** Mirrors `orchestra_agents::AgentKind` ids. */
export type AgentId = "claude_code" | "codex" | "copilot" | "antigravity";
export const AGENT_IDS: AgentId[] = ["claude_code", "codex", "copilot", "antigravity"];

export type Readiness = "ready" | "needs_login" | "needs_download" | "not_installed" | "error";

export type AuthMethodInfo = {
  id: string;
  name: string;
  description: string | null;
  terminal_command: string | null;
};

/** Mirrors `orchestra_acp::ConfigChoice`. */
export type ConfigChoice = { id: string; name: string; description: string | null; group: string | null };

/** Mirrors `orchestra_acp::ConfigOptionInfo`: a session option the agent exposes. */
export type ConfigOption = {
  id: string;
  name: string;
  description: string | null;
  /** `mode`, `model`, `thought_level`, or the agent's own word. */
  category: string;
  current: string;
  choices: ConfigChoice[];
};

/** Mirrors `orchestra_acp::ModeInfo`. */
export type ModeInfo = { id: string; name: string; description: string | null };

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
    config_options?: ConfigOption[];
    modes?: ModeInfo[];
    default_mode?: string | null;
    /** The mode Orchestra picks when none is chosen. */
    autonomous_mode?: string | null;
  } | null;
  error: string | null;
  login_hint: string;
  install_hint: string;
};

export type Tool = { id: string; title: string; toolKind: string; status: string };

/** The turn as it happened: prose and tool calls in arrival order. */
export type Segment = { kind: "text"; text: string } | { kind: "thought"; text: string } | { kind: "tool"; tool: Tool };

export type TranscriptLine = { ms: number; label: string; text: string; tone: Tone };
export type Tone = "in" | "out" | "ok" | "warn" | "dim";

export type RunStatus = "connecting" | "running" | "done" | "failed";

/** Context accounting from the agent's `usage` updates. */
export type Usage = { used: number; size: number; cost?: number; currency?: string };

export type Run = {
  id: string;
  track: string;
  session: string;
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
  /** Latest context accounting; live runs update it, restored runs get it on hydrate. */
  usage?: Usage;
  /**
   * Below the membrane: what the inspector shows. Empty until `loaded`,
   * which is immediate for live runs and lazy for restored ones.
   */
  loaded: boolean;
  thought: string;
  tools: Tool[];
  transcript: TranscriptLine[];
  /** Ordered prose and tool calls, for showing a turn as it unfolds. */
  segments: Segment[];
};

export type View = "track" | "settings" | "worker" | "new-track" | "edit-track" | "design";

// ----- artifacts: what the human keeps beside tracks and attaches to them —
// designs (a sketch board worked out with an agent), knowledge later -----
export type ArtifactKind = "design" | "knowledge";
export type ArtifactInfo = {
  id: string;
  kind: ArtifactKind;
  title: string;
  agent: string;
  config: OptionConfig;
  color: string;
  tags: string[];
  created_at: number;
  updated_at: number;
};
export type Stroke = { points: [number, number, number][]; color: string; size: number };
export type DesignTag = "" | "goal" | "constraint" | "question" | "idea";
export type DesignNode = {
  id: string;
  kind: "note" | "sketch";
  x: number;
  y: number;
  w: number;
  h: number;
  text: string;
  tag: DesignTag;
  strokes: Stroke[];
  by: string;
};
export type DesignEdge = { id: string; from: string; to: string; label: string; by: string };
export type DesignChange = { id: number; target: string; run: string | null; before: unknown; after: unknown };
export type DesignDoc = { version: number; nodes: DesignNode[]; edges: DesignEdge[]; changes: DesignChange[]; next: number };
/** One edit to a board, as the core applies it. */
export type DesignOp =
  | { op: "create_note"; x: number; y: number; w?: number; h?: number; text?: string; tag?: DesignTag }
  | { op: "add_stroke"; stroke: Stroke }
  | { op: "set_sketch"; id?: string; x: number; y: number; w: number; h: number; strokes: Stroke[] }
  | { op: "update"; id: string; text?: string; tag?: DesignTag }
  | { op: "move"; id: string; x: number; y: number; w?: number; h?: number }
  | { op: "delete"; ids: string[] }
  | { op: "connect"; from: string; to: string; label?: string };
export type DesignResult = { ok: boolean; id?: string; error?: string };
export const ARTIFACT_SESSION = "artifact";
/** What changed on a design's board, as the core sends it. */
export type DesignDelta = {
  id: string;
  base: number;
  version: number;
  nodes: DesignNode[];
  removed_nodes: string[];
  edges: DesignEdge[];
  removed_edges: string[];
  changes: DesignChange[];
  next: number;
};

/** A board with a delta applied: changed items in place, new ones at the end. */
export function withDelta(doc: DesignDoc, d: DesignDelta): DesignDoc {
  const merge = <T extends { id: string }>(list: T[], changed: T[], removed: string[]): T[] => {
    const byId = new Map(changed.map((x) => [x.id, x]));
    const gone = new Set(removed);
    const kept = list.filter((x) => !gone.has(x.id)).map((x) => byId.get(x.id) ?? x);
    const known = new Set(list.map((x) => x.id));
    return [...kept, ...changed.filter((x) => !known.has(x.id))];
  };
  return {
    version: d.version,
    nodes: merge(doc.nodes, d.nodes, d.removed_nodes),
    edges: merge(doc.edges, d.edges, d.removed_edges),
    changes: d.changes,
    next: d.next,
  };
}

/** Artifact conversations are kept under this key, apart from tracks. */
export const artifactKey = (id: string) => `artifact:${id}`;
const EMPTY_DOC: DesignDoc = { version: 0, nodes: [], edges: [], changes: [], next: 0 };

/** Prefix of conductor prompts Orchestra injects itself (worker reports). Language-neutral. */
export const REPORT_PREFIX = "[worker-report]";
/** Heads the list of files under a human message, as the core stores it. */
export const ATTACH_MARK = "[attachments]";

/** A file waiting in the composer to go with the next message. */
export type Attachment = { path: string; name: string; size: number };

/** A stored prompt split into what the human wrote and the files they attached. */
export function splitAttachments(prompt: string): { text: string; files: string[] } {
  const at = prompt.indexOf(`\n\n${ATTACH_MARK}\n`);
  const head = prompt.startsWith(`${ATTACH_MARK}\n`) ? 0 : at;
  if (head < 0) return { text: prompt, files: [] };
  const list = prompt.slice(head).replace(/^\s*\[attachments\]\n/, "");
  const files = list
    .split("\n")
    .map((l) => l.replace(/^- /, "").trim())
    .filter(Boolean);
  return { text: prompt.slice(0, head), files };
}

/** The message as the core will store it: the text, then its files. */
function withAttachments(text: string, files: string[]): string {
  if (!files.length) return text;
  return `${text}\n\n${ATTACH_MARK}\n${files.map((f) => `- ${f}`).join("\n")}`;
}

/** First line of a turn that hands the conductor a worker's permission question. */
export const PERMISSION_PREFIX = "[worker-permission]";

/** First line of the turn that carries the human's answer to a decision card. */
export const DECISION_PREFIX = "[decision]";

/** A question the conductor put to the human, as the core keeps it. */
export type DecisionOption = { label: string; detail: string; id?: string };
export type DecisionStatus = "open" | "decided" | "dismissed";
export type Decision = {
  id: number;
  track: string;
  /** The conductor run that asked. */
  run: string | null;
  question: string;
  context: string;
  options: DecisionOption[];
  recommended: number | null;
  allow_other: boolean;
  status: DecisionStatus;
  /** Index of the chosen option; null for an answer in the human's own words. */
  choice: number | null;
  answer: string | null;
  note: string;
  created_at: number;
  decided_at: number | null;
  /** Set when the card is an agent's permission question. */
  permission: { session: string; request: string } | null;
};

/** What the user asked for; `system` follows the OS. */
export type ThemePref = "system" | "dark" | "light";

/** Conversation text size. */
export type ChatFont = "s" | "m" | "l";

/** Interface typeface. */
export type UiFont = "sans" | "mono" | "system" | "serif";

/** Zoom bounds in percent; steps of 10, as Ctrl+= / Ctrl+- move. */
export const ZOOM_MIN = 50;
export const ZOOM_MAX = 200;
export const ZOOM_STEP = 10;

/** Settings sections, in the settings column. */
export type SettingsSection = "overview" | "appearance" | "chat" | "agents" | "about";

/** Mirrors `workspace::Entry`: one file or folder, path relative to the track folder. */
export type WsEntry = { path: string; name: string; dir: boolean; size: number };

/** Mirrors `workspace::FileContent`. */
export type WsFile = {
  path: string;
  name: string;
  size: number;
  kind: "markdown" | "text" | "image" | "binary" | "large";
  ext: string;
  text: string | null;
  data_url: string | null;
};

/** Mirrors `workspace::Change`. */
export type WsChange = { path: string; code: string; untracked: boolean; from: string | null };

/** Mirrors `workspace::GitStatus`. */
export type WsGit = { repo: boolean; branch: string; changes: WsChange[] };

/** What the side panel shows: git changes, the file tree, or an open file. */
export type PanelTab = "changes" | "files" | "file";

/** Mirrors the `app_info` command. */
export type AppInfo = {
  version: string;
  db_path: string;
  adapters_dir: string;
  workspace: string;
  runs: number;
};

const scheme = typeof window !== "undefined" ? window.matchMedia("(prefers-color-scheme: dark)") : null;

/** Mirrors the `agent_download` event payload. */
export type DownloadProgress = {
  agent: AgentId;
  phase: "downloading" | "unpacking" | "done";
  received: number;
  total: number | null;
};

/** Everything above the membrane plus, per run, the detail kept below it. */
class Store {
  /** Effective theme class on <html>. */
  theme = $state<"dk" | "lt">("dk");
  /** The stored preference behind `theme`. */
  themePref = $state<ThemePref>("system");
  view = $state<View>("track");
  /** First-run setup, shown in front of the shell. */
  setupOpen = $state(false);
  /** Setup step: 1 agents, 2 appearance. */
  setupStep = $state<1 | 2>(1);
  /** Left rail folded to icons. Persisted. */
  railCollapsed = $state(false);
  /** Second column (tracks and their workers) shown. Persisted. */
  trackListOpen = $state(true);
  /** Which settings section is open. */
  settingsSection = $state<SettingsSection>("overview");
  /** Worker whose session is open in the main area (view === "worker"). */
  openWorker = $state("");
  /** Conversation text size. Persisted. */
  chatFont = $state<ChatFont>("s");
  /** Interface typeface. Persisted. */
  uiFont = $state<UiFont>("sans");
  /** Interface language preference. Persisted; "system" follows the OS. */
  langPref = $state<LangPref>("system");
  /** Native webview zoom in percent. Persisted. */
  zoom = $state(100);
  info = $state<AppInfo | null>(null);
  /** Column widths in px: the rail (expanded), the tracks column, the side panel. Persisted. */
  railWidth = $state(200);
  trackListWidth = $state(264);
  panelWidth = $state(460);

  /** The bottom terminal panel: shown, its height, and whether it was ever
   *  opened (it stays mounted after that so shells survive folding). */
  termOpen = $state(false);
  termHeight = $state(280);
  termMounted = $state(false);

  /** Artifacts of every kind, most recently touched first, and the one open. */
  artifacts = $state<ArtifactInfo[]>([]);
  artifact = $state("");
  /** Replaced whole by each board the core sends; never changed in place. */
  designDoc = $state.raw<DesignDoc>({ ...EMPTY_DOC });
  /** The design whose board `designDoc` holds. */
  designLoaded = $state("");
  /** Width of the conversation beside a design's board. Persisted. */
  artifactChatWidth = $state(460);
  /** The designs column shown. Persisted. */
  designListOpen = $state(true);

  setArtifactChatWidth(px: number, persist = false) {
    this.artifactChatWidth = Math.min(760, Math.max(340, Math.round(px)));
    if (persist) this.persistWidth("artifact_chat_width", this.artifactChatWidth);
  }

  async setDesignList(open: boolean) {
    this.designListOpen = open;
    try {
      await invoke("set_setting", { key: "designlist", value: open ? "open" : "closed" });
    } catch (err) {
      this.lastError = String(err);
    }
  }

  /** Items picked on the board; they go with the next message. */
  designSelected = $state<string[]>([]);
  /** The open artifact's agent session. */
  artifactSession = $state<{ open: boolean; busy: boolean }>({ open: false, busy: false });

  /** Files in the composer, waiting for the next message. */
  /** Files waiting in each conversation's composer (a track's, an artifact's). */
  attachmentsBy = $state<Record<string, Attachment[]>>({});
  /** Design briefs being written out to attach; the composer waits for them. */
  attaching = $state(0);

  /** The conversation on screen, as a key for what waits in its composer. */
  get chatKey(): string {
    return this.chatArtifact ? artifactKey(this.artifact) : `track:${this.track}`;
  }

  /** Files waiting to go with the next message of the conversation on screen. */
  get attachments(): Attachment[] {
    return this.attachmentsBy[this.chatKey] ?? [];
  }

  set attachments(list: Attachment[]) {
    this.attachmentsBy = { ...this.attachmentsBy, [this.chatKey]: list };
  }

  /** Every decision of every track, oldest first. */
  decisions = $state<Decision[]>([]);
  /** The decision whose answer is on its way. */
  answering = $state<number | null>(null);

  /** The side panel on the track: the working folder as files and changes. Persisted. */
  panelOpen = $state(false);
  panelTab = $state<PanelTab>("files");
  /** Open file tabs, relative paths, in opening order; `activeFile` is the one shown. */
  openFiles = $state<string[]>([]);
  activeFile = $state("");
  /** Show the tree beside an open file, as Kiro does. */
  panelTree = $state(true);
  tree = $state<WsEntry[]>([]);
  treeLoading = $state(false);
  git = $state<WsGit | null>(null);
  gitLoading = $state(false);
  /** The change whose diff is shown. */
  diffPath = $state("");
  diffs = $state<Record<string, string>>({});
  files = $state<Record<string, WsFile>>({});
  /** Markdown files render as a preview unless the raw text is asked for. */
  rawMarkdown = $state<Record<string, boolean>>({});

  private persistWidth(key: string, value: number) {
    invoke("set_setting", { key, value: String(value) }).catch((err) => {
      this.lastError = String(err);
    });
  }

  setRailWidth(px: number, persist = false) {
    this.railWidth = Math.min(320, Math.max(160, Math.round(px)));
    if (persist) this.persistWidth("rail_width", this.railWidth);
  }

  setTrackListWidth(px: number, persist = false) {
    this.trackListWidth = Math.min(480, Math.max(200, Math.round(px)));
    if (persist) this.persistWidth("tracklist_width", this.trackListWidth);
  }

  setPanelWidth(px: number, persist = false) {
    const max = Math.max(360, Math.floor(window.innerWidth * 0.7));
    this.panelWidth = Math.min(max, Math.max(320, Math.round(px)));
    if (persist) this.persistWidth("panel_width", this.panelWidth);
  }

  /** Show or hide the side panel; persisted. Opening refreshes it. */
  async setPanel(open: boolean) {
    this.panelOpen = open;
    if (open) void this.refreshWorkspace();
    try {
      await invoke("set_setting", { key: "panel", value: open ? "open" : "closed" });
    } catch (err) {
      this.lastError = String(err);
    }
  }

  /** Forget what the panel loaded; the next open track fills it again. */
  private clearWorkspace() {
    this.tree = [];
    this.git = null;
    this.diffs = {};
    this.diffPath = "";
    this.files = {};
    this.openFiles = [];
    this.activeFile = "";
    if (this.panelTab === "file") this.panelTab = "files";
  }

  /** Reload the tree, the git status and every open file. */
  async refreshWorkspace() {
    if (!this.track) return;
    await Promise.all([this.loadTree(), this.loadGit(), ...this.openFiles.map((p) => this.loadFile(p))]);
    if (this.diffPath) await this.loadDiff(this.diffPath);
  }

  async loadTree() {
    const track = this.track;
    if (!track) return;
    this.treeLoading = true;
    try {
      const entries = await invoke<WsEntry[]>("workspace_tree", { track });
      if (this.track === track) this.tree = entries;
    } catch (err) {
      this.lastError = String(err);
    } finally {
      this.treeLoading = false;
    }
  }

  async loadGit() {
    const track = this.track;
    if (!track) return;
    this.gitLoading = true;
    try {
      const git = await invoke<WsGit>("workspace_git_status", { track });
      if (this.track === track) {
        this.git = git;
        if (this.diffPath && !git.changes.some((c) => c.path === this.diffPath)) this.diffPath = "";
      }
    } catch (err) {
      this.lastError = String(err);
    } finally {
      this.gitLoading = false;
    }
  }

  /** Show one change's diff in the changes tab. */
  async loadDiff(path: string) {
    const track = this.track;
    const change = this.git?.changes.find((c) => c.path === path);
    if (!track || !change) return;
    this.diffPath = path;
    try {
      const text = await invoke<string>("workspace_git_diff", { track, path, untracked: change.untracked });
      if (this.track === track) this.diffs = { ...this.diffs, [path]: text };
    } catch (err) {
      this.lastError = String(err);
    }
  }

  private async loadFile(path: string) {
    const track = this.track;
    if (!track) return;
    try {
      const file = await invoke<WsFile>("workspace_read", { track, path });
      if (this.track === track) this.files = { ...this.files, [path]: file };
    } catch (err) {
      this.lastError = String(err);
    }
  }

  /** Open a file in its own panel tab and show it. */
  async openFile(path: string) {
    if (!this.openFiles.includes(path)) this.openFiles = [...this.openFiles, path];
    this.activeFile = path;
    this.panelTab = "file";
    this.panelOpen = true;
    await this.loadFile(path);
  }

  closeFile(path: string) {
    const i = this.openFiles.indexOf(path);
    this.openFiles = this.openFiles.filter((p) => p !== path);
    const { [path]: _gone, ...rest } = this.files;
    this.files = rest;
    if (this.activeFile === path) {
      const next = this.openFiles[Math.min(i, this.openFiles.length - 1)];
      if (next) this.activeFile = next;
      else {
        this.activeFile = "";
        this.panelTab = "files";
      }
    }
  }

  /** Select a file in the system file manager. */
  async revealFile(path: string) {
    if (!this.track) return;
    try {
      await invoke("workspace_reveal", { track: this.track, path });
    } catch (err) {
      this.lastError = String(err);
    }
  }
  /** Every run of every track; views filter by `track`. */
  runs = $state<Run[]>([]);
  /** Every track, oldest first. */
  tracks = $state<Track[]>([]);
  /** Id of the track in the main area. Persisted. */
  track = $state("");
  /** Track whose tag dialog is open; "" means closed. */
  tagDialog = $state("");
  /** Slash commands the conductor's session offered, by track id. */
  commands = $state<Record<string, SlashCommand[]>>({});
  /** The last list seen per agent id, kept as a setting so the composer can complete before a session opens. */
  commandCache = $state<Record<string, SlashCommand[]>>({});

  /** Commands to complete in the composer: this track's session's, else the agent's last known. */
  /** The conversation on screen is an artifact's, not a track's conductor. */
  get chatArtifact(): boolean {
    return this.view === "design" && !!this.artifact;
  }

  /** The turns of the conversation on screen, oldest first. */
  get chatRuns(): Run[] {
    return this.chatArtifact ? this.artifactRuns : this.trackRuns.filter((r) => r.session === "conductor");
  }

  /** Decision cards of the conversation on screen; artifacts have none. */
  get chatDecisions(): Decision[] {
    if (this.chatArtifact) {
      const key = artifactKey(this.artifact);
      return this.decisions.filter((d) => d.track === key);
    }
    return this.trackDecisions;
  }

  get slashCommands(): SlashCommand[] {
    if (this.chatArtifact) return this.commandCache[this.currentArtifact?.agent ?? ""] ?? [];
    return this.commands[this.track] ?? this.commandCache[this.agent] ?? [];
  }

  /** The conductor session per track: open, busy. Read on track select, never opened by it. */
  conductor = $state<Record<string, ConductorState>>({});
  /** Track whose conductor is being opened right now. */
  conductorOpening = $state("");
  /** A cancel is on its way to the conductor. */
  cancelling = $state(false);

  get conductorState(): ConductorState {
    return this.conductor[this.track] ?? { open: false, busy: false, agent: null, commands: [] };
  }

  private takeConductorState(track: string, state: ConductorState) {
    this.conductor = { ...this.conductor, [track]: state };
    if (state.commands.length) this.rememberCommands(track, state.commands);
  }

  /** Whether a track's conductor session is open (active). */
  isActive(track: string): boolean {
    return !!this.conductor[track]?.open;
  }

  /** Ask how every conductor is, without touching any. Tracks not listed are closed. */
  async refreshConductor() {
    try {
      const list = await invoke<[string, ConductorState][]>("conductor_states");
      const next: Record<string, ConductorState> = {};
      for (const [track, state] of list) {
        next[track] = state;
        if (state.commands.length) this.rememberCommands(track, state.commands);
      }
      this.conductor = next;
    } catch (err) {
      tracing(err);
    }
  }

  /** Open a track's conductor session on purpose (activate): its commands become known and the first message is quick. */
  async openConductor(track = this.track) {
    if (!track || this.conductorOpening) return;
    this.conductorOpening = track;
    this.lastError = "";
    try {
      const state = await invoke<ConductorState>("conductor_open", { track });
      this.takeConductorState(track, state);
    } catch (err) {
      this.lastError = String(err);
    } finally {
      this.conductorOpening = "";
    }
  }

  /** Stop the conductor's turn in flight, as Ctrl+C would; the run ends as cancelled. */
  async cancelConductor() {
    if (this.chatArtifact) {
      this.cancelling = true;
      await this.artifactCancel();
      this.cancelling = false;
      return;
    }
    const track = this.track;
    if (!track || !this.busy) return;
    this.cancelling = true;
    try {
      await invoke("conductor_cancel", { track });
    } catch (err) {
      this.lastError = String(err);
    } finally {
      this.cancelling = false;
    }
  }

  /** Close a track's conductor session (deactivate); it resumes with its memory next time. */
  async closeConductor(track = this.track) {
    if (!track) return;
    this.lastError = "";
    try {
      const state = await invoke<ConductorState>("conductor_close", { track });
      this.takeConductorState(track, state);
    } catch (err) {
      this.lastError = String(err);
    }
  }

  private rememberCommands(track: string, list: SlashCommand[]) {
    this.commands = { ...this.commands, [track]: list };
    const agent = this.tracks.find((t) => t.id === track)?.agent;
    if (!agent) return;
    this.commandCache = { ...this.commandCache, [agent]: list };
    invoke("set_setting", { key: `commands:${agent}`, value: JSON.stringify(list) }).catch((err) => {
      this.lastError = String(err);
    });
  }
  /** The user's tags, each with a colour; persisted as the `tags` setting. */
  tagPool = $state<TagDef[]>([]);
  /** The wall clock, refreshed every half minute, so "today" and "yesterday" stay right. */
  now = $state(Date.now());
  /** The track list's filter, sort and fold. Persisted. */
  trackFilter = $state<TrackFilter>({ ...DEFAULT_TRACK_FILTER });

  /** Change part of the list's filter; persisted. */
  async setTrackFilter(patch: Partial<TrackFilter>) {
    this.trackFilter = { ...this.trackFilter, ...patch };
    try {
      await invoke("set_setting", { key: "tracks_filter", value: JSON.stringify(this.trackFilter) });
    } catch (err) {
      this.lastError = String(err);
    }
  }

  /** A tag's colour, or the neutral label colour for one not in the pool. */
  tagColor(name: string): string {
    return this.tagPool.find((t) => t.name === name)?.color ?? "var(--lab)";
  }

  private async persistTags() {
    try {
      await invoke("set_setting", { key: "tags", value: JSON.stringify(this.tagPool) });
    } catch (err) {
      this.lastError = String(err);
    }
  }

  /** Make a tag with a colour drawn at random, avoiding ones already in use while possible. */
  async createTag(name: string): Promise<TagDef | undefined> {
    const clean = name.trim();
    if (!clean) return undefined;
    const existing = this.tagPool.find((t) => t.name === clean);
    if (existing) return existing;
    const used = new Set(this.tagPool.map((t) => t.color));
    const free = TRACK_COLORS.filter((c) => !used.has(c));
    const pick = free.length ? free : TRACK_COLORS;
    const tag = { name: clean, color: pick[Math.floor(Math.random() * pick.length)] };
    this.tagPool = [...this.tagPool, tag];
    await this.persistTags();
    return tag;
  }

  /** Drop a tag from the pool and from every track carrying it. */
  async deleteTag(name: string) {
    this.tagPool = this.tagPool.filter((t) => t.name !== name);
    await this.persistTags();
    for (const tr of this.tracks) {
      if (tr.tags.includes(name)) await this.updateTrack(tr.id, { tags: tr.tags.filter((x) => x !== name) });
    }
    for (const d of this.artifacts) {
      if (d.tags.includes(name)) await this.updateArtifact(d.id, { tags: d.tags.filter((x) => x !== name) });
    }
  }
  lastError = $state("");
  restored = $state(false);

  /** Last detection result; null until setup has run once. */
  agents = $state<AgentStatus[] | null>(null);
  /** Agent id workers are opened on. */
  agent = $state<AgentId>("claude_code");
  detecting = $state(false);
  /** Agent ids with a login or download in flight. */
  working = $state<Record<string, string>>({});
  /** Live download progress per agent id, while a download runs. */
  downloads = $state<Record<string, DownloadProgress>>({});

  get currentTrack(): Track | undefined {
    return this.tracks.find((t) => t.id === this.track);
  }

  /** Add files to the next message by path; folders and missing paths are skipped. */
  async attach(paths: string[], key = this.chatKey) {
    const waiting = () => this.attachmentsBy[key] ?? [];
    const fresh = paths.filter((p) => !waiting().some((a) => a.path === p));
    if (!fresh.length) return;
    try {
      const stats = await invoke<Attachment[]>("file_stats", { paths: fresh });
      // Into the conversation it was meant for, even if another is on screen now.
      const add = stats.filter((s) => !waiting().some((a) => a.path === s.path));
      this.attachmentsBy = { ...this.attachmentsBy, [key]: [...waiting(), ...add] };
    } catch (err) {
      this.lastError = String(err);
    }
  }

  /** The system file picker, opened in the track's folder. */
  async pickAttachments() {
    try {
      const paths = await invoke<string[]>("pick_files", { start: this.currentTrack?.cwd ?? null });
      await this.attach(paths);
    } catch (err) {
      this.lastError = String(err);
    }
  }

  /** A pasted image (or file) has no path: keep it in the app's folder, then attach it. */
  async attachBlob(blob: Blob, name: string) {
    try {
      const data = await new Promise<string>((resolve, reject) => {
        const reader = new FileReader();
        reader.onload = () => resolve(String(reader.result).replace(/^data:[^,]*,/, ""));
        reader.onerror = () => reject(reader.error);
        reader.readAsDataURL(blob);
      });
      const path = await invoke<string>("save_attachment", { name, data });
      await this.attach([path]);
    } catch (err) {
      this.lastError = String(err);
    }
  }

  detach(path: string) {
    this.attachments = this.attachments.filter((a) => a.path !== path);
  }

  /** A path inside the current track's folder, relative with "/", or null outside it. */
  relativeToTrack(path: string): string | null {
    // An artifact has no folder panel to open files in.
    if (this.chatArtifact) return null;
    const root = this.currentTrack?.cwd;
    if (!root) return null;
    const norm = (p: string) => p.replace(/\\/g, "/").replace(/\/+$/, "");
    const r = norm(root);
    const p = norm(path);
    const same = (a: string, b: string) => a.toLowerCase() === b.toLowerCase();
    if (p.length <= r.length + 1 || !same(p.slice(0, r.length), r) || p[r.length] !== "/") return null;
    return p.slice(r.length + 1);
  }

  /** A track-relative path ("src/a.ts") as an absolute one, in the folder's own separators. */
  absoluteInTrack(rel: string): string {
    const root = this.currentTrack?.cwd ?? "";
    const sep = root.includes("\\") ? "\\" : "/";
    return `${root.replace(/[\\/]+$/, "")}${sep}${rel.split("/").join(sep)}`;
  }

  // ----- artifacts -----

  /** The designs, most recently touched first. */
  get designs(): ArtifactInfo[] {
    return this.artifacts.filter((a) => a.kind === "design").sort((a, b) => b.updated_at - a.updated_at);
  }

  /**
   * Attach a design to the next message of a track: its board as a brief
   * (goals, constraints, questions, notes) and, when it has ink, a picture
   * of it — written out as files and attached like any other.
   */
  async attachDesign(id: string, labels: Record<string, string>) {
    const a = this.artifacts.find((x) => x.id === id);
    if (!a) return;
    const key = this.chatKey;
    this.attaching += 1;
    try {
      const doc = await invoke<DesignDoc>("design_doc", { id });
      const markdown = briefOf(a.title, doc, labels);
      const paths = await invoke<string[]>("export_artifact", { id, markdown, image: boardPng(doc) });
      await this.attach(paths, key);
    } catch (err) {
      this.lastError = String(err);
    } finally {
      this.attaching -= 1;
    }
  }

  get currentArtifact(): ArtifactInfo | undefined {
    return this.artifacts.find((d) => d.id === this.artifact);
  }

  /** The open artifact's conversation, oldest first. */
  get artifactRuns(): Run[] {
    const key = artifactKey(this.artifact);
    return this.runs.filter((r) => r.track === key);
  }

  get artifactBusy(): boolean {
    return this.artifactRuns.some((r) => r.status === "running" || r.status === "connecting");
  }

  async loadArtifacts() {
    try {
      this.artifacts = await invoke<ArtifactInfo[]>("list_artifacts", { kind: null });
    } catch (err) {
      this.lastError = String(err);
    }
  }

  /** Show the designs: the last one open, else the newest. */
  async showDesigns() {
    if (this.view === "design") {
      await this.setDesignList(!this.designListOpen);
      return;
    }
    this.view = "design";
    if (!this.designListOpen) void this.setDesignList(true);
    if (!this.artifacts.length) await this.loadArtifacts();
    const pick = this.designs.find((d) => d.id === this.artifact) ?? this.designs[0];
    if (pick) await this.openArtifact(pick.id);
  }

  async openArtifact(id: string) {
    if (this.artifact !== id) this.designSelected = [];
    this.artifact = id;
    this.view = "design";
    void this.refreshArtifactSession();
    if (this.designLoaded !== id) {
      this.designLoaded = "";
      this.designDoc = { ...EMPTY_DOC };
    }
    try {
      const doc = await invoke<DesignDoc>("design_doc", { id });
      // An event may have brought a newer board meanwhile; keep that one.
      if (this.artifact === id && !(this.designLoaded === id && this.designDoc.version > doc.version)) {
        this.designDoc = doc;
        this.designLoaded = id;
      }
    } catch (err) {
      this.lastError = String(err);
    }
  }

  async createDesign(title: string) {
    const agent = this.readyAgents.some((a) => a.kind === this.agent) ? this.agent : this.readyAgents[0]?.kind;
    if (!agent) {
      this.lastError = "no agent is ready";
      return;
    }
    try {
      const d = await invoke<ArtifactInfo>("create_artifact", { kind: "design", title, agent });
      this.artifacts = [d, ...this.artifacts];
      await this.openArtifact(d.id);
    } catch (err) {
      this.lastError = String(err);
    }
  }

  async updateArtifact(id: string, patch: { title?: string; agent?: string; config?: OptionConfig; color?: string; tags?: string[] }) {
    try {
      const d = await invoke<ArtifactInfo>("update_artifact", { id, ...patch });
      this.artifacts = this.artifacts.map((x) => (x.id === id ? d : x));
    } catch (err) {
      this.lastError = String(err);
    }
  }

  /** Delete an artifact. Returns why the core refused, or "" when it is gone. */
  async deleteArtifact(id: string): Promise<string> {
    try {
      await invoke("delete_artifact", { id });
    } catch (err) {
      return String(err);
    }
    this.artifacts = this.artifacts.filter((d) => d.id !== id);
    this.runs = this.runs.filter((r) => r.track !== artifactKey(id));
    if (this.artifact === id) {
      const next = this.designs[0];
      if (next) await this.openArtifact(next.id);
      else {
        this.artifact = "";
        this.designDoc = { ...EMPTY_DOC };
      }
    }
    return "";
  }

  /** Edit the open board. The core answers with the new board as an event. */
  async designApply(ops: DesignOp[]): Promise<DesignResult[]> {
    const id = this.artifact;
    if (!id || !ops.length) return [];
    try {
      const res = await invoke<DesignResult[]>("design_apply", { id, ops });
      const bad = res.find((r) => !r.ok);
      if (bad) this.lastError = bad.error ?? "the board edit did not go through";
      return res;
    } catch (err) {
      this.lastError = String(err);
      return [];
    }
  }

  async designReview(changes: number[], keep: boolean) {
    if (!this.artifact || !changes.length) return;
    try {
      await invoke("design_review", { id: this.artifact, changes, keep });
    } catch (err) {
      this.lastError = String(err);
    }
  }

  /** Take what changed on a board. It applies to the open board when that
   *  is the version it was made on; a board that missed a change (or is
   *  still loading) is read again whole. */
  takeDesign(delta: DesignDelta) {
    const id = delta.id;
    const d = this.artifacts.find((x) => x.id === id);
    if (d) d.updated_at = Date.now();
    if (id !== this.artifact) return;
    if (this.designLoaded === id && delta.base === this.designDoc.version) {
      this.designDoc = withDelta(this.designDoc, delta);
    } else if (!(this.designLoaded === id && delta.version <= this.designDoc.version)) {
      void this.reloadDesign(id);
    }
  }

  /** Read the open board whole again, keeping whichever is newer. */
  private async reloadDesign(id: string) {
    try {
      const doc = await invoke<DesignDoc>("design_doc", { id });
      if (this.artifact === id && !(this.designLoaded === id && this.designDoc.version >= doc.version)) {
        this.designDoc = doc;
        this.designLoaded = id;
      }
    } catch (err) {
      this.lastError = String(err);
    }
  }

  /** One message to the artifact's agent: the composer's text and files, the
   *  picture of the board when it has ink, and the items picked on it. */
  async artifactSend(prompt: string) {
    const id = this.artifact;
    const d = this.currentArtifact;
    const typed = prompt.trim();
    const key = this.chatKey;
    const files = this.attachments.map((a) => a.path);
    const selected = [...this.designSelected];
    if (!id || !d || this.artifactBusy || (!typed && !files.length && !selected.length)) return;
    this.lastError = "";
    this.attachments = [];
    const image = boardPng(this.designDoc);
    const text = withAttachments(typed, files);
    const pending: Run = {
      id: `pending-${Date.now()}`,
      track: artifactKey(id),
      session: ARTIFACT_SESSION,
      agent: d.agent,
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
      segments: [],
    };
    const pendingId = pending.id;
    this.runs.push(pending);
    try {
      const run = await invoke<string>("artifact_prompt", { id, text: typed, image, selected, lang: i18n.lang, files });
      const r = this.runs.find((x) => x.id === pendingId || x.id === run);
      if (r) r.id = run;
      void this.refreshArtifactSession();
    } catch (err) {
      this.runs = this.runs.filter((x) => x.id !== pendingId);
      if (!(this.attachmentsBy[key] ?? []).length) void this.attach(files, key);
      this.lastError = String(err);
    }
  }

  async refreshArtifactSession() {
    const id = this.artifact;
    if (!id) return;
    try {
      const s = await invoke<{ open: boolean; busy: boolean }>("artifact_state", { id });
      if (this.artifact === id) this.artifactSession = s;
    } catch {
      // Cosmetic.
    }
  }

  async artifactCancel() {
    if (this.artifact) await invoke("artifact_cancel", { id: this.artifact }).catch(() => {});
  }

  /** Take a decision the core sent or returned, new or changed. */
  upsertDecision(d: Decision) {
    const i = this.decisions.findIndex((x) => x.id === d.id);
    if (i >= 0) this.decisions[i] = d;
    else this.decisions.push(d);
  }

  /** The current track's decisions, oldest first. */
  get trackDecisions(): Decision[] {
    return this.decisions.filter((d) => d.track === this.track);
  }

  /** How many of a track's decisions wait on the human. */
  openDecisions(track: string): number {
    return this.decisions.filter((d) => d.track === track && d.status === "open").length;
  }

  /** Answer a decision card; the core hands the answer to the conductor. */
  async answerDecision(id: number, choice: number | null, answer: string | null, note: string): Promise<boolean> {
    this.answering = id;
    try {
      this.upsertDecision(await invoke<Decision>("answer_decision", { id, choice, answer, note }));
      return true;
    } catch (err) {
      this.lastError = String(err);
      return false;
    } finally {
      this.answering = null;
    }
  }

  /** Set a decision aside; the conductor is not told. */
  async dismissDecision(id: number) {
    try {
      this.upsertDecision(await invoke<Decision>("dismiss_decision", { id }));
    } catch (err) {
      this.lastError = String(err);
    }
  }

  /** The current track's runs, oldest first. */
  get trackRuns(): Run[] {
    return this.runs.filter((r) => r.track === this.track);
  }

  get activeRun(): Run | undefined {
    const runs = this.chatArtifact ? this.artifactRuns : this.trackRuns;
    return runs.find((r) => r.status === "running" || r.status === "connecting");
  }

  /** The conversation on screen has a turn in flight; the composer waits. */
  get busy(): boolean {
    return this.chatRuns.some((r) => r.status === "running" || r.status === "connecting");
  }

  /** Any track has a run in flight; the brand mark pulses. */
  get anyLive(): boolean {
    return this.runs.some((r) => r.status === "running" || r.status === "connecting");
  }

  /** Create a track and open it. */
  async createTrack(patch: TrackPatch): Promise<boolean> {
    this.lastError = "";
    try {
      const track = await invoke<Track>("create_track", { patch });
      this.tracks.push(track);
      await this.selectTrack(track.id);
      return true;
    } catch (err) {
      this.lastError = String(err);
      return false;
    }
  }

  /** Show a track in the main area; persisted so the app reopens on it. */
  async selectTrack(id: string) {
    const track = this.tracks.find((t) => t.id === id);
    if (!track) return;
    const changed = this.track !== id;
    this.track = id;
    this.view = "track";
    if (changed) {
      this.clearWorkspace();
      if (this.panelOpen) void this.refreshWorkspace();
      void this.refreshConductor();
    }
    if (this.readyAgents.some((a) => a.kind === track.agent)) this.agent = track.agent as AgentId;
    try {
      await invoke("set_setting", { key: "track", value: id });
    } catch (err) {
      this.lastError = String(err);
    }
  }

  /** Change a track's fields; the conductor picks up agent and option changes at its next message. */
  async updateTrack(id: string, patch: TrackPatch): Promise<boolean> {
    this.lastError = "";
    try {
      const next = await invoke<Track>("update_track", { id, patch });
      this.tracks = this.tracks.map((t) => (t.id === id ? next : t));
      if (id === this.track && this.readyAgents.some((a) => a.kind === next.agent)) this.agent = next.agent as AgentId;
      return true;
    } catch (err) {
      this.lastError = String(err);
      return false;
    }
  }

  /** Move the current track's conductor to another agent; it reopens there on the next message. */
  async setTrackAgent(agent: AgentId) {
    this.agent = agent;
    if (this.track) await this.updateTrack(this.track, { agent });
  }

  /** Delete a track with its runs and memory; the app moves to a neighbour or to creation. */
  /** Delete a track. Returns why the core refused, or "" when it is gone. */
  async deleteTrack(id: string): Promise<string> {
    this.lastError = "";
    try {
      await invoke("delete_track", { id });
    } catch (err) {
      return String(err);
    }
    this.decisions = this.decisions.filter((d) => d.track !== id);
    this.tracks = this.tracks.filter((t) => t.id !== id);
    this.runs = this.runs.filter((r) => r.track !== id);
    if (this.track === id) {
      const next = this.tracks.at(-1);
      if (next) await this.selectTrack(next.id);
      else {
        this.track = "";
        this.view = "new-track";
      }
    }
    return "";
  }

  /** Native folder picker; empty when cancelled. */
  async pickFolder(start: string): Promise<string> {
    try {
      return (await invoke<string | null>("pick_folder", { start })) ?? "";
    } catch (err) {
      this.lastError = String(err);
      return "";
    }
  }

  get readyAgents(): AgentStatus[] {
    return (this.agents ?? []).filter((a) => a.readiness === "ready");
  }

  get currentAgent(): AgentStatus | undefined {
    return this.agents?.find((a) => a.kind === this.agent);
  }

  /** The select options an agent advertised at detection (mode, model, effort, …). */
  optionsOf(agent: string): ConfigOption[] {
    return (this.agents?.find((a) => a.kind === agent)?.probe?.config_options ?? []).filter((o) => o.choices.length > 0);
  }

  /** The mode Orchestra picks for an agent when the track chooses none. */
  autonomousModeOf(agent: string): string {
    const status = this.agents?.find((a) => a.kind === agent);
    return status?.probe?.autonomous_mode ?? status?.probe?.config_options?.find((o) => o.category === "mode")?.current ?? "";
  }

  /** Value in effect for one of an agent's options: the track's choice, else the agent's current. */
  effective(agent: string, config: OptionConfig, option: ConfigOption): string {
    const chosen = config[option.id];
    if (chosen) return chosen;
    if (option.category === "mode") return this.autonomousModeOf(agent) || option.current;
    return option.current;
  }

  /** Human name of a choice, falling back to its id. */
  choiceName(option: ConfigOption | undefined, id: string): string {
    return option?.choices.find((c) => c.id === id)?.name ?? id;
  }

  /** Context accounting to show: the current track's conductor, live or its latest run that reported one. */
  get context(): Usage | undefined {
    const runs = this.chatRuns;
    const live = runs.find((r) => r.status === "running" || r.status === "connecting");
    return live?.usage ?? [...runs].reverse().find((r) => r.usage)?.usage;
  }

  /** The agent a role runs on in the current track. */
  roleAgent(role: Role): string {
    if (this.chatArtifact) return this.currentArtifact?.agent ?? this.agent;
    const track = this.currentTrack;
    if (!track) return this.agent;
    return role === "conductor" ? track.agent : workerAgentOf(track);
  }

  /** A role's chosen options in the current track. */
  roleConfig(role: Role): OptionConfig {
    if (this.chatArtifact) return this.currentArtifact?.config ?? {};
    const track = this.currentTrack;
    if (!track) return {};
    return role === "conductor" ? track.conductor_config : workerConfigOf(track);
  }

  /** Put a role on an agent. Takes effect at the next session. A worker
   *  moved onto the agent it already runs on keeps its options. */
  async setRoleAgent(role: Role, agent: string) {
    const d = this.chatArtifact ? this.currentArtifact : undefined;
    if (d) {
      // Options are in one agent's terms; another agent starts from its own.
      await this.updateArtifact(d.id, agent === d.agent ? { agent } : { agent, config: {} });
      return;
    }
    const track = this.currentTrack;
    if (!track) return;
    if (role === "conductor") {
      this.agent = agent as AgentId;
      await this.updateTrack(track.id, { agent });
    } else {
      const same = agent === workerAgentOf(track);
      await this.updateTrack(track.id, { worker_agent: agent, worker_config: same ? workerConfigOf(track) : {} });
    }
  }

  /** Set one of a role's options on the current track; empty clears it. Takes effect at the next session. */
  async setRoleOption(role: Role, id: string, value: string) {
    const d = this.chatArtifact ? this.currentArtifact : undefined;
    if (d) {
      const { [id]: _old, ...rest } = d.config;
      await this.updateArtifact(d.id, { config: value ? { ...rest, [id]: value } : rest });
      return;
    }
    const track = this.currentTrack;
    if (!track) return;
    const { [id]: _old, ...rest } = this.roleConfig(role);
    const next = value ? { ...rest, [id]: value } : rest;
    await this.updateTrack(track.id, role === "conductor" ? { conductor_config: next } : { worker_config: next });
  }

  /** Apply the preference to <html> and, for `system`, follow the OS. */
  private applyTheme() {
    const dark = this.themePref === "system" ? (scheme?.matches ?? true) : this.themePref === "dark";
    this.theme = dark ? "dk" : "lt";
    document.documentElement.className = this.theme;
  }

  /** Choose a theme; persisted, applied at once. */
  async setTheme(pref: ThemePref) {
    this.themePref = pref;
    this.applyTheme();
    try {
      await invoke("set_setting", { key: "theme", value: pref });
    } catch (err) {
      this.lastError = String(err);
    }
  }

  /** Title-bar shortcut: flip between the explicit themes. */
  toggleTheme() {
    void this.setTheme(this.theme === "dk" ? "light" : "dark");
  }

  /** Fold or unfold the rail; persisted. */
  async setRail(collapsed: boolean) {
    this.railCollapsed = collapsed;
    try {
      await invoke("set_setting", { key: "rail", value: collapsed ? "collapsed" : "expanded" });
    } catch (err) {
      this.lastError = String(err);
    }
  }

  /** Show a worker's session: the conductor's messages and the worker's replies. */
  async openWorkerView(name: string) {
    this.openWorker = name;
    this.view = "worker";
    // Restored runs only carry their folded text; replay them for the full turn.
    await Promise.all(this.trackRuns.filter((r) => r.session === name && !r.loaded).map((r) => this.hydrate(r)));
  }

  /** Open settings on a section. */
  openSettings(section: SettingsSection = "overview") {
    this.settingsSection = section;
    this.view = "settings";
    void this.loadInfo();
  }

  async loadInfo() {
    try {
      this.info = await invoke<AppInfo>("app_info");
    } catch (err) {
      this.lastError = String(err);
    }
  }

  /** The language in effect. */
  get lang(): Lang {
    return i18n.lang;
  }

  /** Choose the interface language; persisted, applied at once. */
  async setLang(pref: LangPref) {
    this.langPref = pref;
    i18n.lang = pref === "system" ? systemLang() : pref;
    document.documentElement.lang = i18n.lang;
    try {
      await invoke("set_setting", { key: "language", value: pref });
    } catch (err) {
      this.lastError = String(err);
    }
  }

  /** Interface typeface; persisted, applied through a root attribute. */
  async setUiFont(font: UiFont) {
    this.uiFont = font;
    document.documentElement.dataset.uiFont = font;
    try {
      await invoke("set_setting", { key: "ui_font", value: font });
    } catch (err) {
      this.lastError = String(err);
    }
  }

  /**
   * Zoom the whole interface, like Ctrl+= / Ctrl+- in a browser. Uses the
   * webview's native zoom; if that is refused, CSS zoom on the root.
   */
  async setZoom(percent: number, persist = true) {
    const z = Math.min(ZOOM_MAX, Math.max(ZOOM_MIN, Math.round(percent / ZOOM_STEP) * ZOOM_STEP));
    this.zoom = z;
    try {
      await getCurrentWebview().setZoom(z / 100);
      (document.documentElement.style as unknown as { zoom: string }).zoom = "";
    } catch {
      (document.documentElement.style as unknown as { zoom: string }).zoom = `${z}%`;
    }
    if (!persist) return;
    try {
      await invoke("set_setting", { key: "zoom", value: String(z) });
    } catch (err) {
      this.lastError = String(err);
    }
  }

  /** Conversation text size; persisted, applied through a root attribute. */
  async setChatFont(size: ChatFont) {
    this.chatFont = size;
    document.documentElement.dataset.chatFont = size;
    try {
      await invoke("set_setting", { key: "chat_font", value: size });
    } catch (err) {
      this.lastError = String(err);
    }
  }

  /** Show or hide the track list column; persisted. */
  async setTerminal(open: boolean) {
    this.termOpen = open;
    if (open) this.termMounted = true;
    try {
      await invoke("set_setting", { key: "terminal", value: open ? "open" : "closed" });
    } catch (err) {
      this.lastError = String(err);
    }
  }

  setTermHeight(px: number, persist = false) {
    const max = Math.max(160, window.innerHeight - 220);
    this.termHeight = Math.min(max, Math.max(120, Math.round(px)));
    if (persist) this.persistWidth("terminal_height", this.termHeight);
  }

  async setTrackList(open: boolean) {
    this.trackListOpen = open;
    try {
      await invoke("set_setting", { key: "tracklist", value: open ? "open" : "closed" });
    } catch (err) {
      this.lastError = String(err);
    }
  }

  /** Rebuild the timeline, agent list and preferences from the store. Called once at startup. */
  async restore() {
    scheme?.addEventListener("change", () => {
      if (this.themePref === "system") this.applyTheme();
    });
    try {
      const [summaries, tracks, savedTrack, agents, theme, rail, tracklist, chatFont, panelWidth, railWidth, trackListWidth, uiFont, zoom, language, panel, tagsJson, ...commandJson] =
        await Promise.all([
        invoke<RunSummary[]>("list_runs"),
        invoke<Track[]>("list_tracks"),
        invoke<string | null>("get_setting", { key: "track" }),
        invoke<AgentStatus[] | null>("agent_statuses"),
        invoke<string | null>("get_setting", { key: "theme" }),
        invoke<string | null>("get_setting", { key: "rail" }),
        invoke<string | null>("get_setting", { key: "tracklist" }),
        invoke<string | null>("get_setting", { key: "chat_font" }),
        invoke<string | null>("get_setting", { key: "panel_width" }),
        invoke<string | null>("get_setting", { key: "rail_width" }),
        invoke<string | null>("get_setting", { key: "tracklist_width" }),
        invoke<string | null>("get_setting", { key: "ui_font" }),
        invoke<string | null>("get_setting", { key: "zoom" }),
        invoke<string | null>("get_setting", { key: "language" }),
        invoke<string | null>("get_setting", { key: "panel" }),
        invoke<string | null>("get_setting", { key: "tags" }),
        ...AGENT_IDS.map((id) => invoke<string | null>("get_setting", { key: `commands:${id}` })),
      ]);
      try {
        const saved = await invoke<string | null>("get_setting", { key: "tracks_filter" });
        if (saved) this.trackFilter = { ...DEFAULT_TRACK_FILTER, ...(JSON.parse(saved) as Partial<TrackFilter>) };
      } catch {
        this.trackFilter = { ...DEFAULT_TRACK_FILTER };
      }
      setInterval(() => (this.now = Date.now()), 30_000);
      const cache: Record<string, SlashCommand[]> = {};
      AGENT_IDS.forEach((id, i) => {
        try {
          const list = commandJson[i] ? (JSON.parse(commandJson[i] as string) as SlashCommand[]) : [];
          if (Array.isArray(list)) cache[id] = list;
        } catch {
          // A stale cache is no cache.
        }
      });
      this.commandCache = cache;
      if (language === "system" || language === "ko" || language === "en") this.langPref = language;
      i18n.lang = this.langPref === "system" ? systemLang() : this.langPref;
      document.documentElement.lang = i18n.lang;
      if (uiFont === "sans" || uiFont === "mono" || uiFont === "system" || uiFont === "serif") this.uiFont = uiFont;
      document.documentElement.dataset.uiFont = this.uiFont;
      const z = Number(zoom);
      if (Number.isFinite(z) && z >= ZOOM_MIN && z <= ZOOM_MAX && z !== 100) void this.setZoom(z, false);
      const w = Number(panelWidth);
      if (Number.isFinite(w) && w > 0) this.setPanelWidth(w);
      this.panelOpen = panel === "open";
      try {
        const pool = tagsJson ? (JSON.parse(tagsJson) as unknown) : [];
        if (Array.isArray(pool)) this.tagPool = pool.filter((t) => t && typeof t.name === "string" && typeof t.color === "string");
      } catch {
        this.tagPool = [];
      }
      // Tags on tracks that predate the pool (or lost it) still get a colour.
      for (const tr of tracks) for (const tag of tr.tags) if (!this.tagPool.some((t) => t.name === tag)) void this.createTag(tag);
      const rw = Number(railWidth);
      if (Number.isFinite(rw) && rw > 0) this.setRailWidth(rw);
      const tw = Number(trackListWidth);
      if (Number.isFinite(tw) && tw > 0) this.setTrackListWidth(tw);
      if (theme === "system" || theme === "dark" || theme === "light") this.themePref = theme;
      this.applyTheme();
      this.railCollapsed = rail === "collapsed";
      this.trackListOpen = tracklist !== "closed";
      if (chatFont === "s" || chatFont === "m" || chatFont === "l") this.chatFont = chatFont;
      document.documentElement.dataset.chatFont = this.chatFont;
      // The core closes runs left live by a previous process, so nothing
      // restored can be in flight.
      this.runs = summaries.map(fromSummary);
      this.tracks = tracks;
      void this.loadArtifacts();
      try {
        this.decisions = await invoke<Decision[]>("list_decisions", { track: null });
      } catch (err) {
        this.lastError = String(err);
      }
      try {
        const [term, termHeight, artifactChat, designList] = await Promise.all([
          invoke<string | null>("get_setting", { key: "terminal" }),
          invoke<string | null>("get_setting", { key: "terminal_height" }),
          invoke<string | null>("get_setting", { key: "artifact_chat_width" }),
          invoke<string | null>("get_setting", { key: "designlist" }),
        ]);
        const dc = Number(artifactChat);
        if (Number.isFinite(dc) && dc > 0) this.setArtifactChatWidth(dc);
        this.designListOpen = designList !== "closed";
        const h = Number(termHeight);
        if (Number.isFinite(h) && h > 0) this.setTermHeight(h);
        if (term === "open") void this.setTerminal(true);
      } catch {
        // Defaults are fine.
      }
      this.agents = agents;
      this.pickDefaultAgent();
      // Reopen on the track that was open, else the newest; none means the
      // first screen is creating one.
      const current = tracks.find((t) => t.id === savedTrack) ?? tracks.at(-1);
      if (current) {
        this.track = current.id;
        if (this.readyAgents.some((a) => a.kind === current.agent)) this.agent = current.agent as AgentId;
        if (this.panelOpen) void this.refreshWorkspace();
        void this.refreshConductor();
      } else {
        this.view = "new-track";
      }
      // Setup comes first when nothing has been detected yet, or when the
      // last detection left nothing to run workers on.
      if (agents === null || this.readyAgents.length === 0) {
        this.setupStep = 1;
        this.setupOpen = true;
      }
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
    await this.workOn(agent, t("agents.loggingIn"), () => invoke<AgentStatus>("login_agent", { agent, method: method ?? null }));
  }

  /** Fetch the agent's ACP server (Antigravity). Progress arrives as events. */
  async download(agent: AgentId) {
    this.downloads = { ...this.downloads, [agent]: { agent, phase: "downloading", received: 0, total: null } };
    try {
      await this.workOn(agent, t("agents.downloading"), () => invoke<AgentStatus>("download_agent", { agent }));
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
      run.segments = [];
      for (const e of events) fold(run, e.at_ms, e.event);
      run.loaded = true;
    } catch (err) {
      this.lastError = String(err);
    }
  }

  /**
   * Send a message to the conductor on the selected agent. The conductor
   * decides whether to answer or to open workers; worker runs arrive as
   * `agent` events with their own run ids and are added when first seen.
   */
  async send(prompt: string) {
    if (this.chatArtifact) return this.artifactSend(prompt);
    const typed = prompt.trim();
    const track = this.track;
    const key = this.chatKey;
    const files = this.attachments.map((a) => a.path);
    if ((!typed && !files.length) || !track || this.busy) return;
    this.lastError = "";
    const agent = this.agent;
    // The files go with this message; the composer starts empty again.
    this.attachments = [];
    const text = withAttachments(typed, files);

    // Show the message the moment Enter is pressed. The run gets its real id
    // when the core answers; until then it carries a pending id, and events
    // that arrive for the real id first are routed to it by `apply`.
    const pending: Run = {
      id: `pending-${Date.now()}`,
      track,
      session: "conductor",
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
      segments: [],
    };
    // The array hands back proxies, never the object pushed; match by id.
    const pendingId = pending.id;
    this.runs.push(pending);

    try {
      const id = await invoke<string>("conductor_prompt", { track, prompt: typed, agent, lang: i18n.lang, files });
      const run = this.runs.find((r) => r.id === pendingId || r.id === id);
      if (run) {
        run.id = id;
        run.track = track;
        run.session = "conductor";
        run.agent = agent;
        run.prompt = text;
      }
      // The conductor now runs on this agent; keep the track's record in step.
      const tr = this.tracks.find((t) => t.id === track);
      if (tr && tr.agent !== agent) tr.agent = agent;
    } catch (err) {
      this.runs = this.runs.filter((r) => r.id !== pendingId);
      if (!(this.attachmentsBy[key] ?? []).length) void this.attach(files, key);
      this.lastError = String(err);
    }
  }

  /** Fold one live agent event into the run it belongs to, creating worker runs on first sight. */
  apply(env: Envelope) {
    let run = this.runs.find((r) => r.id === env.run);
    if (!run && (env.session === "conductor" || env.session === ARTIFACT_SESSION)) {
      // The conductor (or artifact agent) turn we just sent, still waiting for its id.
      run = this.runs.find((r) => r.track === env.track && r.session === env.session && r.id.startsWith("pending-"));
      if (run) run.id = env.run;
    }
    if (!run) {
      // A worker the conductor opened, or a report turn Orchestra injected:
      // the core registered it, we have not. Show it now; the prompt text
      // comes with the summary right after.
      run = {
        id: env.run,
        track: env.track,
        session: env.session,
        agent: this.tracks.find((t) => t.id === env.track)?.agent ?? this.agent,
        prompt: "",
        status: "connecting",
        startedAt: Date.now(),
        message: "",
        plan: [],
        toolCount: 0,
        loaded: true,
        thought: "",
        tools: [],
        transcript: [],
        segments: [],
      };
      this.runs.push(run);
      void this.refreshRun(env.run);
    }
    if (env.event.kind === "commands" && env.session === "conductor") this.rememberCommands(env.track, env.event.commands);
    if (env.session === ARTIFACT_SESSION && (env.event.kind === "started" || env.event.kind === "finished" || env.event.kind === "failed")) {
      void this.refreshArtifactSession();
    }
    if (env.session === "conductor" && (env.event.kind === "started" || env.event.kind === "finished" || env.event.kind === "failed")) {
      void this.refreshConductor();
    }
    fold(run, env.at_ms, env.event);
    // A finished run may have written files: the open panel catches up.
    if (this.panelOpen && env.track === this.track && (env.event.kind === "finished" || env.event.kind === "failed")) {
      void this.refreshWorkspace();
    }
  }

  /** Fill a worker run's prompt and agent from the store once it exists there. */
  private async refreshRun(id: string) {
    try {
      const summaries = await invoke<RunSummary[]>("list_runs");
      const s = summaries.find((x) => x.id === id);
      const run = this.runs.find((r) => r.id === id);
      if (s && run) {
        run.prompt = s.prompt;
        run.agent = s.agent;
        run.startedAt = s.started_at;
      }
    } catch {
      // Cosmetic; the next restore fills it in.
    }
  }

  /** Worker names seen in a track, in first-seen order, excluding the conductor. */
  workerNamesIn(track: string): string[] {
    const seen: string[] = [];
    for (const r of this.runs) if (r.track === track && r.session !== "conductor" && !seen.includes(r.session)) seen.push(r.session);
    return seen;
  }

  get workerNames(): string[] {
    return this.workerNamesIn(this.track);
  }
}

/** Pure state transition: the same fold serves live events and replay. */
function fold(run: Run, ms: number, ev: AgentEvent) {
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
    case "message": {
      run.message += ev.text;
      const last = run.segments.at(-1);
      if (last && last.kind === "text") last.text += ev.text;
      else run.segments.push({ kind: "text", text: ev.text });
      break;
    }
    case "thought": {
      run.thought += ev.text;
      const last = run.segments.at(-1);
      if (last && last.kind === "thought") last.text += ev.text;
      else run.segments.push({ kind: "thought", text: ev.text });
      break;
    }
    case "tool_call": {
      const tool: Tool = { id: ev.id, title: ev.title, toolKind: ev.tool_kind, status: ev.status };
      run.tools.push(tool);
      run.segments.push({ kind: "tool", tool });
      run.toolCount = run.tools.length;
      push(run, ms, ev.tool_kind, ev.title, "dim");
      break;
    }
    case "tool_update": {
      const tool = run.tools.find((t) => t.id === ev.id);
      if (tool) tool.status = ev.status;
      const seg = run.segments.find((s) => s.kind === "tool" && s.tool.id === ev.id);
      if (seg && seg.kind === "tool") seg.tool.status = ev.status;
      break;
    }
    case "plan":
      run.plan = ev.entries;
      push(run, ms, "plan", `${ev.entries.length} entries`, "dim");
      break;
    case "commands":
      push(run, ms, "commands", `${ev.commands.length} slash commands`, "dim");
      break;
    case "usage": {
      const raw = ev.raw as { used?: number; size?: number; cost?: { amount?: number; currency?: string } } | null;
      if (raw && typeof raw.used === "number" && typeof raw.size === "number") {
        run.usage = {
          used: raw.used,
          size: raw.size,
          cost: raw.cost?.amount ?? run.usage?.cost,
          currency: raw.cost?.currency ?? run.usage?.currency,
        };
      }
      break;
    }
    case "permission":
      push(run, ms, "asks", `${ev.tool_kind} · ${ev.title}`, "warn");
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
    track: s.track,
    session: s.session,
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
    segments: [],
  };
}

function push(run: Run, ms: number, label: string, text: string, tone: Tone) {
  run.transcript.push({ ms, label, text, tone });
}

/** A quiet log for failures that have a visible fallback. */
function tracing(err: unknown) {
  console.warn(err);
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

/** Subscribe once; the core emits one `agent` event per agent event. */
export async function connectEvents() {
  await Promise.all([
    listen<Envelope>("agent", (e) => store.apply(e.payload)),
    listen<DownloadProgress>("agent_download", (e) => store.progress(e.payload)),
    listen<Decision>("decision", (e) => store.upsertDecision(e.payload)),
    listen<DesignDelta>("design", (e) => store.takeDesign(e.payload)),
  ]);
}
