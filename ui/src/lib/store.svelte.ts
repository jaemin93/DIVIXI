import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { i18n, systemLang, t, type Lang, type LangPref } from "./i18n.svelte";

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
  | { kind: "commands"; commands: SlashCommand[] }
  | { kind: "finished"; stop_reason: string }
  | { kind: "failed"; error: string };

/** Mirrors `orchestra_core::SlashCommand`: one `/name` the agent offers. */
export type SlashCommand = { name: string; description: string; hint: string | null };

/** Mirrors `conductor::ConductorState`. */
export type ConductorState = { open: boolean; busy: boolean; agent: string | null; commands: SlashCommand[] };

export type Envelope = {
  track: string;
  lane: string;
  run: string;
  at_ms: number;
  event: LaneEvent;
};

/** Session options chosen for one role: `option id → value id`, in the agent's own terms. */
export type OptionConfig = Record<string, string>;

/** Mirrors `orchestra_store::TrackInfo`: one conductor, its lanes, one folder. */
export type Track = {
  id: string;
  name: string;
  intent: string;
  cwd: string;
  /** Agent the conductor runs on. */
  agent: string;
  conductor_config: OptionConfig;
  /** Agent lanes run on; empty means the conductor's. */
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

/** The agent a track's lanes run on. */
export function laneAgentOf(track: Track): string {
  return track.worker_agent || track.agent;
}

/** The options lanes open with: the worker's own, else (never set apart) the conductor's. */
export function laneConfigOf(track: Track): OptionConfig {
  const own = track.worker_agent !== "" || Object.keys(track.worker_config).length > 0;
  return own ? track.worker_config : track.conductor_config;
}

/** Who a session setting is for. */
export type Role = "conductor" | "worker";

/** Mirrors `orchestra_store::RunSummary`: one run as the timeline sees it. */
export type RunSummary = {
  id: string;
  track: string;
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

export type View = "track" | "settings" | "lane" | "new-track" | "edit-track";

/** Prefix of conductor prompts Orchestra injects itself (lane reports). Language-neutral. */
export const REPORT_PREFIX = "[lane-report]";
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

/** First line of the turn that carries the human's answer to a decision card. */
export const DECISION_PREFIX = "[decision]";

/** A question the conductor put to the human, as the core keeps it. */
export type DecisionOption = { label: string; detail: string };
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
  /** Second column (tracks and their lanes) shown. Persisted. */
  trackListOpen = $state(true);
  /** Which settings section is open. */
  settingsSection = $state<SettingsSection>("overview");
  /** Lane whose session is open in the main area (view === "lane"). */
  openLane = $state("");
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

  /** Files in the composer, waiting for the next message. */
  attachments = $state<Attachment[]>([]);

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
  get slashCommands(): SlashCommand[] {
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
  }
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

  get currentTrack(): Track | undefined {
    return this.tracks.find((t) => t.id === this.track);
  }

  /** Add files to the next message by path; folders and missing paths are skipped. */
  async attach(paths: string[]) {
    const fresh = paths.filter((p) => !this.attachments.some((a) => a.path === p));
    if (!fresh.length) return;
    try {
      const stats = await invoke<Attachment[]>("file_stats", { paths: fresh });
      for (const s of stats) if (!this.attachments.some((a) => a.path === s.path)) this.attachments.push(s);
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
    return this.trackRuns.find((r) => r.status === "running" || r.status === "connecting");
  }

  /** The current track's conductor has a turn in flight; the composer waits. */
  get busy(): boolean {
    return this.trackRuns.some((r) => r.lane === "conductor" && (r.status === "running" || r.status === "connecting"));
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
    const runs = this.trackRuns.filter((r) => r.lane === "conductor");
    const live = runs.find((r) => r.status === "running" || r.status === "connecting");
    return live?.usage ?? [...runs].reverse().find((r) => r.usage)?.usage;
  }

  /** The agent a role runs on in the current track. */
  roleAgent(role: Role): string {
    const track = this.currentTrack;
    if (!track) return this.agent;
    return role === "conductor" ? track.agent : laneAgentOf(track);
  }

  /** A role's chosen options in the current track. */
  roleConfig(role: Role): OptionConfig {
    const track = this.currentTrack;
    if (!track) return {};
    return role === "conductor" ? track.conductor_config : laneConfigOf(track);
  }

  /** Put a role on an agent. Takes effect at the next session. A worker
   *  moved onto the agent it already runs on keeps its options. */
  async setRoleAgent(role: Role, agent: string) {
    const track = this.currentTrack;
    if (!track) return;
    if (role === "conductor") {
      this.agent = agent as AgentId;
      await this.updateTrack(track.id, { agent });
    } else {
      const same = agent === laneAgentOf(track);
      await this.updateTrack(track.id, { worker_agent: agent, worker_config: same ? laneConfigOf(track) : {} });
    }
  }

  /** Set one of a role's options on the current track; empty clears it. Takes effect at the next session. */
  async setRoleOption(role: Role, id: string, value: string) {
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

  /** Show a lane's session: the conductor's messages and the worker's replies. */
  async openLaneView(name: string) {
    this.openLane = name;
    this.view = "lane";
    // Restored runs only carry their folded text; replay them for the full turn.
    await Promise.all(this.trackRuns.filter((r) => r.lane === name && !r.loaded).map((r) => this.hydrate(r)));
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
      try {
        this.decisions = await invoke<Decision[]>("list_decisions", { track: null });
      } catch (err) {
        this.lastError = String(err);
      }
      try {
        const [term, termHeight] = await Promise.all([
          invoke<string | null>("get_setting", { key: "terminal" }),
          invoke<string | null>("get_setting", { key: "terminal_height" }),
        ]);
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
      // last detection left nothing to run lanes on.
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
   * decides whether to answer or to open lanes; lane runs arrive as
   * `lane` events with their own run ids and are added when first seen.
   */
  async send(prompt: string) {
    const typed = prompt.trim();
    const track = this.track;
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
      lane: "conductor",
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
        run.lane = "conductor";
        run.agent = agent;
        run.prompt = text;
      }
      // The conductor now runs on this agent; keep the track's record in step.
      const tr = this.tracks.find((t) => t.id === track);
      if (tr && tr.agent !== agent) tr.agent = agent;
    } catch (err) {
      this.runs = this.runs.filter((r) => r.id !== pendingId);
      if (!this.attachments.length) void this.attach(files);
      this.lastError = String(err);
    }
  }

  /** Fold one live lane event into the run it belongs to, creating lane runs on first sight. */
  apply(env: Envelope) {
    let run = this.runs.find((r) => r.id === env.run);
    if (!run && env.lane === "conductor") {
      // The conductor turn we just sent, still waiting for its id.
      run = this.runs.find((r) => r.track === env.track && r.lane === "conductor" && r.id.startsWith("pending-"));
      if (run) run.id = env.run;
    }
    if (!run) {
      // A lane the conductor opened, or a report turn Orchestra injected:
      // the core registered it, we have not. Show it now; the prompt text
      // comes with the summary right after.
      run = {
        id: env.run,
        track: env.track,
        lane: env.lane,
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
    if (env.event.kind === "commands" && env.lane === "conductor") this.rememberCommands(env.track, env.event.commands);
    if (env.lane === "conductor" && (env.event.kind === "started" || env.event.kind === "finished" || env.event.kind === "failed")) {
      void this.refreshConductor();
    }
    fold(run, env.at_ms, env.event);
    // A finished run may have written files: the open panel catches up.
    if (this.panelOpen && env.track === this.track && (env.event.kind === "finished" || env.event.kind === "failed")) {
      void this.refreshWorkspace();
    }
  }

  /** Fill a lane run's prompt and agent from the store once it exists there. */
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

  /** Lane names seen in a track, in first-seen order, excluding the conductor. */
  laneNamesIn(track: string): string[] {
    const seen: string[] = [];
    for (const r of this.runs) if (r.track === track && r.lane !== "conductor" && !seen.includes(r.lane)) seen.push(r.lane);
    return seen;
  }

  get laneNames(): string[] {
    return this.laneNamesIn(this.track);
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

/** Subscribe once; the core emits one `lane` event per lane event. */
export async function connectEvents() {
  await Promise.all([
    listen<Envelope>("lane", (e) => store.apply(e.payload)),
    listen<DownloadProgress>("agent_download", (e) => store.progress(e.payload)),
    listen<Decision>("decision", (e) => store.upsertDecision(e.payload)),
  ]);
}
